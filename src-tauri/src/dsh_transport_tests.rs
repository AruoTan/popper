use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;

fn provider(base: String) -> ProviderConfig {
    ProviderConfig {
        id: "dsh".into(),
        name: "DSH".into(),
        kind: crate::models::ProviderKind::Dsh,
        dsh_workspace: Some("/workspace".into()),
        enabled: true,
        base_url: base,
        models: vec![],
    }
}
fn plan() -> ThinkingRequestPlan {
    ThinkingRequestPlan {
        capability: None,
        levels: vec![],
        mode: ThinkingMode::Off,
    }
}
fn catalog() -> Value {
    json!({"groups":[
        {"id":"one", "name":"One", "models":[{"id":"same", "name":"Same", "reasoning":{"efforts":[{"id":"none"},{"id":"low"},{"id":"high"}]}}]},
        {"id":"two", "name":"Two", "models":[{"id":"same", "name":"Same"},{"id":"org/model", "name":"Namespaced"}]}
    ]})
}

#[test]
fn catalog_retains_provider_identity_and_native_reasoning_metadata() {
    let models = catalog_models(&catalog()).unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["one/same", "two/same", "two/org/model"]
    );
    assert_eq!(
        models[0].thinking_levels,
        [ThinkingLevel::Low, ThinkingLevel::High]
    );
    assert!(models[0].thinking_capability.as_ref().unwrap().supports_off);
    assert!(models[1].thinking_levels.is_empty());
    assert_eq!(
        split_model_id("two/org/model").unwrap(),
        ("two", "org/model")
    );
    assert!(split_model_id("same").is_err());
    assert!(split_model_id("one/").is_err());
    assert_eq!(
        catalog_models(&json!({"groups":[]})).unwrap_err().code,
        "DSH_NO_MODELS"
    );
}

#[test]
fn durable_message_completes_missing_suffix_without_duplicating_live_text() {
    let mut output = Output::default();
    let mut text = String::new();
    let mut delta = |part: String| {
        text.push_str(&part);
        ControlFlow::Continue(())
    };
    let mut reasoning = |_| ControlFlow::Continue(());
    for value in [
        json!({"type":"assistant-stream", "frame":{"type":"start","attemptId":"a"}}),
        json!({"type":"assistant-stream", "frame":{"type":"chunk","attemptId":"a","index":0,"chunk":{"type":"text-delta","text":"你"}}}),
        json!({"type":"event","event":{"type":"assistant/message","data":{"message":{"content":[{"type":"text","text":"你好"}]}}}}),
        json!({"type":"assistant-stream","frame":{"type":"end","outcome":{"kind":"committed"}}}),
    ] {
        assert!(!output.follow(&value, &mut delta, &mut reasoning).unwrap());
    }
    assert!(output.follow(&json!({"type":"event","event":{"type":"turn/end","data":{"reason":{"kind":"completed"}}}}), &mut delta, &mut reasoning).unwrap());
    assert_eq!(text, "你好");
    assert_eq!(output.content, "你好");
}

#[test]
fn gaps_failed_turns_and_abandoned_prefixes_are_not_success() {
    let mut output = Output::default();
    let mut delta = |_| ControlFlow::Continue(());
    let mut reasoning = |_| ControlFlow::Continue(());
    output
        .follow(
            &json!({"type":"assistant-stream","frame":{"type":"start","attemptId":"a"}}),
            &mut delta,
            &mut reasoning,
        )
        .unwrap();
    assert!(output.follow(&json!({"type":"assistant-stream","frame":{"type":"chunk","attemptId":"a","index":2,"chunk":{"type":"text-delta","text":"gap"}}}), &mut delta, &mut reasoning).is_err());
    assert_eq!(output.follow(&json!({"type":"event","event":{"type":"turn/end","data":{"reason":{"kind":"error"}}}}), &mut delta, &mut reasoning).unwrap_err().code, "DSH_TURN_ERROR");
    output.current = "uncommitted".into();
    assert_eq!(output.follow(&json!({"type":"assistant-stream","frame":{"type":"end","outcome":{"kind":"abandoned"}}}), &mut delta, &mut reasoning).unwrap_err().code, "DSH_ATTEMPT_ABANDONED");
}

#[derive(Clone, Debug)]
struct MockUpload {
    session_id: String,
    name: String,
    bytes: Vec<u8>,
    receipt: String,
}

struct MockGateway {
    base: String,
    calls: Arc<Mutex<Vec<Value>>>,
    prompt: Arc<Semaphore>,
    commands: Arc<Mutex<Vec<Value>>>,
    sockets: Arc<std::sync::atomic::AtomicUsize>,
    signals: tokio::sync::broadcast::Sender<MockSignal>,
    ack_gate: Arc<Semaphore>,
    reject_auth: Arc<std::sync::atomic::AtomicBool>,
    stale_follow_events: Arc<std::sync::atomic::AtomicBool>,
    reject_websocket: Arc<std::sync::atomic::AtomicU16>,
    uploads: Arc<Mutex<Vec<MockUpload>>>,
    upload_status: Arc<std::sync::atomic::AtomicU16>,
    upload_response: Arc<Mutex<Option<String>>>,
    upload_seen: Arc<Semaphore>,
    upload_blocked: Arc<std::sync::atomic::AtomicBool>,
    upload_gate: Arc<Semaphore>,
    save_response: Arc<Mutex<Option<(u16, Value)>>>,
    accept: tokio::task::JoinHandle<()>,
}
#[derive(Clone, Debug)]
enum MockSignal {
    Prompt(String, String),
    Disconnect,
}
impl Drop for MockGateway {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

async fn mock_gateway(hold: bool, rpc_error: bool) -> MockGateway {
    mock_gateway_auth(hold, rpc_error, false).await
}
async fn mock_gateway_auth(hold: bool, rpc_error: bool, cookie_login: bool) -> MockGateway {
    mock_gateway_options(hold, rpc_error, cookie_login, false).await
}
async fn mock_gateway_options(
    hold: bool,
    rpc_error: bool,
    cookie_login: bool,
    slow_ack: bool,
) -> MockGateway {
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(vec![]));
    let prompt = Arc::new(Semaphore::new(0));
    let commands = Arc::new(Mutex::new(vec![]));
    let sockets = Arc::new(AtomicUsize::new(0));
    let sessions = Arc::new(AtomicUsize::new(0));
    let ack_gate = Arc::new(Semaphore::new(0));
    let reject_auth = Arc::new(AtomicBool::new(false));
    let stale_follow_events = Arc::new(AtomicBool::new(false));
    let reject_websocket = Arc::new(AtomicU16::new(0));
    let (signals, _) = tokio::sync::broadcast::channel(32);
    let uploads = Arc::new(Mutex::new(Vec::<MockUpload>::new()));
    let upload_status = Arc::new(AtomicU16::new(0));
    let upload_response = Arc::new(Mutex::new(None::<String>));
    let upload_seen = Arc::new(Semaphore::new(0));
    let upload_blocked = Arc::new(AtomicBool::new(false));
    let upload_gate = Arc::new(Semaphore::new(0));
    let save_response = Arc::new(Mutex::new(None::<(u16, Value)>));
    let seen_save_response = save_response.clone();
    let seen_uploads = uploads.clone();
    let seen_upload_status = upload_status.clone();
    let seen_upload_response = upload_response.clone();
    let seen_upload_seen = upload_seen.clone();
    let seen_upload_blocked = upload_blocked.clone();
    let seen_upload_gate = upload_gate.clone();
    let seen_calls = calls.clone();
    let seen_prompt = prompt.clone();
    let seen_commands = commands.clone();
    let seen_sockets = sockets.clone();
    let send_signals = signals.clone();
    let seen_ack_gate = ack_gate.clone();
    let rejected = reject_auth.clone();
    let stale_events = stale_follow_events.clone();
    let rejected_websocket = reject_websocket.clone();
    let accept = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let uploads = seen_uploads.clone();
            let upload_status = seen_upload_status.clone();
            let upload_response = seen_upload_response.clone();
            let upload_seen = seen_upload_seen.clone();
            let upload_blocked = seen_upload_blocked.clone();
            let upload_gate = seen_upload_gate.clone();
            let save_response = seen_save_response.clone();
            let calls = seen_calls.clone();
            let prompt = seen_prompt.clone();
            let commands = seen_commands.clone();
            let sockets = seen_sockets.clone();
            let sessions = sessions.clone();
            let signals = send_signals.clone();
            let ack_gate = seen_ack_gate.clone();
            let reject_auth = rejected.clone();
            let stale_events = stale_events.clone();
            let reject_websocket = rejected_websocket.clone();
            tokio::spawn(async move {
                loop {
                    let mut prefix = [0; 6];
                    loop {
                        let count = stream.peek(&mut prefix).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        if count == prefix.len() {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                    if &prefix == b"GET /a" {
                        let socket = accept_hdr_async(
                        stream,
                        |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                         response| {
                            assert_eq!(request.uri().path(), "/api/remote.mux");
                            assert_eq!(request.headers()["x-dsh-client"], "app");
                            if cookie_login {
                                assert!(!request.headers().contains_key("authorization"));
                                assert_eq!(request.headers()["cookie"], "dsh-session=test-cookie");
                            } else {
                                assert!(matches!(
                                    request.headers()["authorization"].to_str().unwrap(),
                                    "Bearer test-dsh-key" | "Bearer rotated-key"
                                ));
                            }
                            let status = reject_websocket.swap(0, Ordering::SeqCst);
                            if status != 0 {
                                return Err(
                                    tokio_tungstenite::tungstenite::http::Response::builder()
                                        .status(status)
                                        .header("set-cookie", "secret-cookie=do-not-print")
                                        .body(Some("secret-response test-dsh-key".into()))
                                        .unwrap(),
                                );
                            }
                            Ok(response)
                        },
                    )
                    .await;
                        let Ok(mut socket) = socket else {
                            return;
                        };
                        let socket_id = sockets.fetch_add(1, Ordering::SeqCst) + 1;
                        let mut received = signals.subscribe();
                        let mut follows = std::collections::HashMap::new();
                        loop {
                            tokio::select! {
                                frame = socket.next() => {
                                    let Some(Ok(Message::Text(text))) = frame else { break; };
                                    let command: Value = serde_json::from_str(&text).unwrap();
                                    commands.lock().unwrap().push(command.clone());
                                    let id = command["streamId"].as_str().unwrap().to_owned();
                                    if command["type"] == "cancel" { follows.remove(&id); continue; }
                                    if command["endpoint"] == "$events" {
                                        socket.send(Message::Text(json!({"type":"item","streamId":"events","value":{"type":"ready","clientId":format!("client-{socket_id}")}}).to_string().into())).await.unwrap();
                                    } else {
                                        assert_eq!(command["endpoint"], "session/follow");
                                        let session = command["payload"]["args"]["request"]["address"]["sessionId"].as_str().unwrap().to_owned();
                                        follows.insert(id.clone(), session);
                                        socket.send(Message::Text(json!({"type":"item","streamId":id,"value":{"type":"snapshot","cursor":-1,"records":[]}}).to_string().into())).await.unwrap();
                                        if stale_events.load(Ordering::SeqCst) {
                                            for value in [
                                                json!({"type":"event","event":{"type":"user/message","data":{"source":{"rpcId":"previous-request"}}}}),
                                                json!({"type":"event","event":{"type":"assistant/message","data":{"message":{"content":[{"type":"text","text":"Old answer must not leak"}]}}}}),
                                                json!({"type":"event","event":{"type":"turn/end","data":{"reason":{"kind":"completed"}}}}),
                                            ] {
                                                socket.send(Message::Text(json!({"type":"item","streamId":id,"value":value}).to_string().into())).await.unwrap();
                                            }
                                        }
                                    }
                                }
                                signal = received.recv() => {
                                    match signal.unwrap() {
                                        MockSignal::Disconnect => { let _ = socket.close(None).await; break; }
                                        MockSignal::Prompt(session, request_id) => {
                                            if hold && !request_id.starts_with("unblocked-") { continue; }
                                            let Some(stream_id) = follows.iter().find(|(_, current)| **current == session).map(|(id, _)| id.clone()) else { continue; };
                                            let text = if session == "remote-session" { "你好" } else { "第二轮" };
                                            let frames = vec![
                                                (stream_id.clone(), json!({"type":"event","event":{"type":"turn/start","time":1000,"data":{"turn":1}}})),
                                                (stream_id.clone(), json!({"type":"event","event":{"type":"user/message","time":1001,"data":{"source":{"kind":"user","rpcId":request_id}}}})),
                                                ("events".to_owned(), json!({"type":"waterfall","event":"approval/request","eventId":format!("approve-{request_id}"),"agentId":session,"request":{}})),
                                                ("events".to_owned(), json!({"type":"waterfall","event":"approval/request","eventId":format!("other-{request_id}"),"agentId":"other-session","request":{}})),
                                                (stream_id.clone(), json!({"type":"assistant-stream","frame":{"type":"start","attemptId":"a"}})),
                                                (stream_id.clone(), json!({"type":"assistant-stream","frame":{"type":"chunk","attemptId":"a","index":0,"chunk":{"type":"reasoning-delta","text":"思考"}}})),
                                                (stream_id.clone(), json!({"type":"assistant-stream","frame":{"type":"chunk","attemptId":"a","index":1,"chunk":{"type":"text-delta","text":text}}})),
                                                (stream_id.clone(), json!({"type":"event","event":{"type":"assistant/message","data":{"message":{"content":[{"type":"text","text":text}]}}}})),
                                                (stream_id.clone(), json!({"type":"assistant-stream","frame":{"type":"end","outcome":{"kind":"committed"}}})),
                                                (stream_id, json!({"type":"event","event":{"type":"turn/end","time":5000,"data":{"reason":{"kind":"completed"}}}})),
                                            ];
                                            for (id, value) in frames {
                                                if socket.send(Message::Text(json!({"type":"item","streamId":id,"value":value}).to_string().into())).await.is_err() { return; }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        return;
                    }
                    let mut bytes = Vec::new();
                    let body_start = loop {
                        let mut chunk = [0; 1024];
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..count]);
                        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            break index + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..body_start].to_vec()).unwrap();
                    if !headers.starts_with("GET / HTTP") {
                        assert!(headers.to_lowercase().contains("x-dsh-client: app\r\n"));
                    }
                    if headers.starts_with("GET / HTTP") {
                        assert!(cookie_login);
                        assert!(!headers.to_lowercase().contains("authorization:"));
                        calls.lock().unwrap().push(json!({"method":"local-login"}));
                        let response = if rpc_error {
                            "HTTP/1.1 303 See Other\r\nLocation: http://untrusted.invalid/\r\nContent-Length: 0\r\n\r\n"
                        } else {
                            "HTTP/1.1 303 See Other\r\nLocation: ./\r\nSet-Cookie: dsh-session=test-cookie; Path=/; HttpOnly; SameSite=Strict\r\nContent-Length: 0\r\n\r\n"
                        };
                        if stream.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                        continue;
                    }
                    if cookie_login {
                        assert!(!headers.to_lowercase().contains("authorization:"));
                        assert!(headers
                            .to_lowercase()
                            .contains("cookie: dsh-session=test-cookie"));
                        assert!(!headers.to_lowercase().contains("httponly"));
                    } else {
                        assert!(
                            headers
                                .to_lowercase()
                                .contains("authorization: bearer test-dsh-key")
                                || headers
                                    .to_lowercase()
                                    .contains("authorization: bearer rotated-key")
                        );
                    }
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    while bytes.len() < body_start + length {
                        let mut chunk = [0; 1024];
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    if headers.starts_with("POST /api/session/uploadFileBinary?") {
                        assert!(headers
                            .to_lowercase()
                            .contains("content-type: application/octet-stream"));
                        let target = headers
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap();
                        let target = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                        let query: std::collections::HashMap<_, _> =
                            target.query_pairs().into_owned().collect();
                        let session_id = query["sessionId"].clone();
                        let name = query["name"].clone();
                        let receipt = {
                            let mut uploads = uploads.lock().unwrap();
                            let receipt = format!("receipt-{session_id}-{}", uploads.len() + 1);
                            uploads.push(MockUpload {
                                session_id: session_id.clone(),
                                name: name.clone(),
                                bytes: bytes[body_start..body_start + length].to_vec(),
                                receipt: receipt.clone(),
                            });
                            receipt
                        };
                        calls.lock().unwrap().push(json!({"method":"session/uploadFileBinary", "sessionId":session_id, "name":name}));
                        upload_seen.add_permits(1);
                        let status = upload_status.swap(0, Ordering::SeqCst);
                        if status != 0 {
                            let response = format!(
                                "HTTP/1.1 {status} Upload Failed\r\nContent-Length: 0\r\n\r\n"
                            );
                            if stream.write_all(response.as_bytes()).await.is_err() {
                                return;
                            }
                            continue;
                        }
                        if upload_blocked.load(Ordering::SeqCst) {
                            let _permit = upload_gate.acquire().await.unwrap();
                        }
                        let body = upload_response.lock().unwrap().clone().unwrap_or_else(|| json!({"ok":true,"value":{"receiptId":receipt,"file":{"attachmentId":"sha256:mock-file","name":name,"bytes":length}}}).to_string());
                        if body == "disconnect-before-headers" {
                            return;
                        }
                        if body == "disconnect-during-body" {
                            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{").await;
                            return;
                        }
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                        if stream.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                        continue;
                    }
                    let envelope: Value =
                        serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
                    let method = envelope["method"].as_str().unwrap();
                    assert!(headers.starts_with(&format!("POST /api/{method} HTTP/1.1")));
                    calls.lock().unwrap().push(envelope.clone());
                    if reject_auth.swap(false, Ordering::SeqCst) {
                        if stream
                            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                    let value = match method {
                        "workspace/create" => {
                            json!({"workspace":{"workspaceId":format!("workspace:{}", envelope["payload"]["args"]["request"]["path"].as_str().unwrap())}})
                        }
                        "session/create" => {
                            if let Some(id) = envelope["payload"]["args"]["request"]["sessionId"].as_str() {
                                assert!(envelope["payload"]["args"]["request"].get("workspaceId").is_none());
                                json!({"sessionId":id})
                            } else {
                                let number = sessions.fetch_add(1, Ordering::SeqCst);
                                json!({"sessionId":if number == 0 { "remote-session".to_owned() } else { format!("remote-session-{}", number+1) }})
                            }
                        }
                        "temporary/save" => json!({
                            "sessionId":envelope["payload"]["args"]["request"]["sessionId"],
                            "workspaceId":"normal-workspace", "savedSessionIds":[envelope["payload"]["args"]["request"]["sessionId"]],
                            "temporary":false, "reconnectRequired":true,
                        }),
                        "session/modelCatalog" => catalog(),
                        "session/prompt" => {
                            let request = &envelope["payload"]["args"]["request"];
                            for part in request["content"].as_array().unwrap() {
                                if part["type"] == "file" {
                                    let uploads = uploads.lock().unwrap();
                                    assert!(uploads.iter().any(|upload| upload.receipt
                                        == part["receiptId"].as_str().unwrap()
                                        && upload.session_id
                                            == request["sessionId"].as_str().unwrap()));
                                }
                            }
                            let _ = signals.send(MockSignal::Prompt(
                                request["sessionId"].as_str().unwrap().to_owned(),
                                request["requestId"].as_str().unwrap().to_owned(),
                            ));
                            prompt.add_permits(2);
                            if slow_ack {
                                let _ = ack_gate.acquire().await;
                            }
                            json!({"accepted":true})
                        }
                        _ => json!({}),
                    };
                    let mut status = 200;
                    let result = if method == "temporary/save" && save_response.lock().unwrap().is_some() {
                        let custom = save_response.lock().unwrap().clone().unwrap();
                        status = custom.0;
                        custom.1
                    } else if rpc_error {
                        json!({"ok":false,"error":{"message":"bad test-dsh-key"}})
                    } else {
                        json!({"ok":true,"value":value})
                    };
                    let body =
                        json!({"type":"server-response","rpcId":envelope["rpcId"],"result":result})
                            .to_string();
                    let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    MockGateway {
        base,
        calls,
        prompt,
        commands,
        sockets,
        signals,
        ack_gate,
        reject_auth,
        stale_follow_events,
        reject_websocket,
        uploads,
        upload_status,
        upload_response,
        upload_seen,
        upload_blocked,
        upload_gate,
        save_response,
        accept,
    }
}
async fn wait_for_calls(gateway: &MockGateway, method: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if gateway
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call["method"] == method)
                .count()
                >= count
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn native_gateway_routes_streams_delegates_interactions_and_deduplicates() {
    for cookie_login in [false, true] {
        let gateway = mock_gateway_auth(false, false, cookie_login).await;
        let key = if cookie_login { "" } else { "test-dsh-key" };
        let provider = provider(gateway.base.clone());
        let models = DshService::new()
            .unwrap()
            .fetch_models(&provider, key)
            .await
            .unwrap();
        assert_eq!(models.len(), 3);
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut notices = vec![];
        let result = DshService::new()
            .unwrap()
            .generate(
                "test-request",
                &provider,
                key,
                "two/org/model",
                &plan(),
                &json!([{"role":"system","content":"translate"},{"role":"user","content":"hello"}]),
                &CancellationToken::new(),
                &GenerationBudget::new(tokio::time::Instant::now(), Duration::from_secs(5)),
                |part| {
                    text.push_str(&part);
                    ControlFlow::Continue(())
                },
                |part| {
                    reasoning.push_str(&part);
                    ControlFlow::Continue(())
                },
                |message| notices.push(message),
            )
            .await
            .unwrap();
        assert_eq!(text, "你好");
        assert_eq!(result.content, "你好");
        assert_eq!(reasoning, "思考");
        assert!(notices.iter().any(|text| text.contains("需要审批")));
        wait_for_calls(&gateway, "$events/result", 2).await;
        let calls = gateway.calls.lock().unwrap();
        let select = calls
            .iter()
            .find(|call| call["method"] == "session/selectModel")
            .unwrap();
        assert_eq!(select["payload"]["args"]["request"]["provider"], "two");
        assert_eq!(select["payload"]["args"]["request"]["model"], "org/model");
        let outcomes: Vec<_> = calls
            .iter()
            .filter(|call| call["method"] == "$events/result")
            .collect();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes
            .iter()
            .all(|call| call["payload"]["args"]["outcome"]["kind"] == "next"));
    }
}

#[tokio::test]
async fn stopping_generation_cancels_the_native_remote_agent() {
    let gateway = mock_gateway(true, false).await;
    let cancellation = CancellationToken::new();
    let stop = cancellation.clone();
    let prompt = gateway.prompt.clone();
    tokio::spawn(async move {
        let _permit = prompt.acquire().await.unwrap();
        stop.cancel();
    });
    let result = DshService::new()
        .unwrap()
        .generate(
            "test-request",
            &provider(gateway.base.clone()),
            "test-dsh-key",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"hello"}]),
            &cancellation,
            &GenerationBudget::new(tokio::time::Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(result.code, "CANCELLED");
    let calls = gateway.calls.lock().unwrap();
    let cancel = calls
        .iter()
        .find(|call| call["method"] == "session/cancel")
        .unwrap();
    assert_eq!(
        cancel["payload"]["args"]["request"]["sessionId"],
        "remote-session"
    );
}

#[tokio::test]
async fn rpc_business_errors_are_checked_and_credentials_are_redacted() {
    let gateway = mock_gateway(false, true).await;
    let error = DshService::new()
        .unwrap()
        .fetch_models(&provider(gateway.base.clone()), "test-dsh-key")
        .await
        .unwrap_err();
    assert_eq!(error.code, "DSH_RPC_ERROR");
    assert!(!error.message.contains("test-dsh-key"));
    assert!(error.message.contains("[redacted]"));
}

#[test]
fn only_local_dsh_addresses_can_omit_the_gateway_key() {
    for base in [
        "http://localhost:3080",
        "https://LOCALHOST:3080",
        "http://127.0.0.1:3080",
        "http://[::1]:3080",
    ] {
        let mut config = provider(base.to_owned());
        assert!(!config.requires_api_key(), "{base}");
        assert!(DshService::new()
            .unwrap()
            .gateway(&provider(base.to_owned()), "")
            .is_ok());
        config.kind = crate::models::ProviderKind::Openai;
        assert!(config.requires_api_key());
    }
    for base in [
        "http://localhost.example:3080",
        "http://localhost@remote.example",
        "http://192.168.1.1:3080",
        "http://remote.example",
        "http://localhost:3080/?next=remote",
        "file://localhost",
    ] {
        assert!(provider(base.to_owned()).requires_api_key(), "{base}");
        assert_eq!(
            DshService::new()
                .unwrap()
                .gateway(&provider(base.to_owned()), "")
                .err()
                .unwrap()
                .code,
            "DSH_API_KEY_MISSING"
        );
    }
    assert_eq!(
        safe_message(&json!({"message":"local failure"}), ""),
        "local failure"
    );
}

#[tokio::test]
async fn local_cookie_login_authenticates_http_websocket_and_remote_cancellation() {
    let gateway = mock_gateway_auth(true, false, true).await;
    let config = provider(gateway.base.clone());
    assert_eq!(
        DshService::new()
            .unwrap()
            .fetch_models(&config, "")
            .await
            .unwrap()
            .len(),
        3
    );
    let cancellation = CancellationToken::new();
    let stop = cancellation.clone();
    let prompt = gateway.prompt.clone();
    tokio::spawn(async move {
        let _permit = prompt.acquire().await.unwrap();
        stop.cancel();
    });
    let error = DshService::new()
        .unwrap()
        .generate(
            "test-request",
            &config,
            "",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"hello"}]),
            &cancellation,
            &GenerationBudget::new(tokio::time::Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        2
    );
    assert!(calls.iter().any(|call| call["method"] == "session/cancel"));
}

#[tokio::test]
async fn local_login_without_a_cookie_fails_without_following_redirects() {
    let gateway = mock_gateway_auth(false, true, true).await;
    let error = DshService::new()
        .unwrap()
        .fetch_models(&provider(gateway.base.clone()), "")
        .await
        .unwrap_err();
    assert_eq!(error.code, "DSH_LOCAL_LOGIN_FAILED");
    assert!(error.message.contains("本地自动登录"));
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
}

async fn run_turn(
    service: &DshService,
    config: &ProviderConfig,
    key: &str,
    request_id: &str,
    model: &str,
    cancellation: &CancellationToken,
) -> Result<TransportSuccess, TransportFailure> {
    service
        .generate(
            request_id,
            config,
            key,
            model,
            &plan(),
            &json!([{"role":"user","content":"hello"}]),
            cancellation,
            &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
}

#[tokio::test]
async fn websocket_rejection_reports_status_without_response_secrets_or_submitting_a_prompt() {
    use std::sync::atomic::Ordering;
    for (status, hint) in [
        (403, "DSH_PUBLIC_URL"),
        (404, "/api/remote.mux"),
        (502, "上游"),
    ] {
        let gateway = mock_gateway_auth(false, false, true).await;
        gateway.reject_websocket.store(status, Ordering::SeqCst);
        let service = DshService::new().unwrap();
        let config = provider(gateway.base.clone());
        assert!(!service.fetch_models(&config, "").await.unwrap().is_empty());
        let error = run_turn(
            &service,
            &config,
            "",
            "rejected-handshake",
            "one/same",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "DSH_WEBSOCKET_HTTP_ERROR");
        assert_eq!(error.status, Some(status));
        assert!(error.message.contains(hint));
        assert!(!error.message.contains("secret"));
        assert!(!error.message.contains("test-dsh-key"));
        assert!(!gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["method"] == "session/prompt"));
    }
}

#[tokio::test]
async fn websocket_cookie_rejection_reauthenticates_once_and_uses_a_new_socket() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    gateway.reject_websocket.store(401, Ordering::SeqCst);
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let mux = service.gateway(&config, "").unwrap().mux().await.unwrap().0;
    assert!(!mux.is_closed());
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
    assert_eq!(
        gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        2
    );
}

#[tokio::test]
async fn connection_check_detects_a_rejected_stream_even_when_model_catalog_works() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    assert_eq!(service.fetch_models(&config, "").await.unwrap().len(), 3);
    gateway.reject_websocket.store(403, Ordering::SeqCst);
    let error = service.check_connection(&config, "").await.unwrap_err();
    assert_eq!(error.status, Some(403));
    assert_eq!(
        service.check_connection(&config, "").await.unwrap().len(),
        3
    );
    let calls = gateway.calls.lock().unwrap();
    assert!(calls.iter().all(|call| matches!(
        call["method"].as_str(),
        Some("local-login" | "session/modelCatalog")
    )));
    drop(calls);
    tokio::time::timeout(Duration::from_secs(2), async {
        while gateway.signals.receiver_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn localhost_connects_to_an_ipv4_published_gateway_without_a_key() {
    let gateway = mock_gateway_auth(false, false, true).await;
    let config = provider(gateway.base.replace("127.0.0.1", "localhost"));
    let result = run_turn(
        &DshService::new().unwrap(),
        &config,
        "",
        "localhost-request",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "你好");
}

#[tokio::test]
async fn model_catalog_and_websocket_follow_the_same_http_proxy_route() {
    let gateway = mock_gateway(false, false).await;
    // A direct localhost dial succeeds at TCP and never completes a handshake;
    // HTTP reaches the healthy gateway through the configured proxy instead.
    let blocked = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let blocked_port = blocked.local_addr().unwrap().port();
    let blocker = tokio::spawn(async move {
        let (stream, _) = blocked.accept().await.unwrap();
        let _stream = stream;
        std::future::pending::<()>().await;
    });
    let upstream = url::Url::parse(&gateway.base).unwrap();
    let upstream_addr = (
        upstream.host_str().unwrap().to_owned(),
        upstream.port().unwrap(),
    );
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let proxy_requests = Arc::new(Mutex::new(vec![]));
    let requests = proxy_requests.clone();
    let relay = tokio::spawn(async move {
        loop {
            let (mut incoming, _) = proxy.accept().await.unwrap();
            let upstream_addr = upstream_addr.clone();
            let requests = requests.clone();
            tokio::spawn(async move {
                let mut headers = Vec::new();
                let mut byte = [0; 1];
                while !headers.ends_with(b"\r\n\r\n") {
                    if incoming.read(&mut byte).await.unwrap() == 0 {
                        return;
                    }
                    headers.push(byte[0]);
                }
                let headers = String::from_utf8(headers).unwrap();
                let (line, rest) = headers.split_once("\r\n").unwrap();
                requests.lock().unwrap().push(line.to_owned());
                let parts = line.split_whitespace().collect::<Vec<_>>();
                let target = url::Url::parse(parts[1]).unwrap();
                assert_eq!(target.host_str(), Some("localhost"));
                let mut outgoing = tokio::net::TcpStream::connect(upstream_addr).await.unwrap();
                outgoing
                    .write_all(
                        format!("{} {} {}\r\n{}", parts[0], target.path(), parts[2], rest)
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
            });
        }
    });
    let client = reqwest::Client::builder()
        .http1_only()
        .pool_max_idle_per_host(0)
        .redirect(reqwest::redirect::Policy::none())
        .proxy(reqwest::Proxy::http(proxy_url).unwrap())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let service = DshService::with_client(client);
    let config = provider(format!("http://localhost:{blocked_port}"));
    assert_eq!(
        service
            .fetch_models(&config, "test-dsh-key")
            .await
            .unwrap()
            .len(),
        3
    );
    let result = service.check_connection(&config, "test-dsh-key").await;
    if result.is_ok() {
        let output = run_turn(
            &service,
            &config,
            "test-dsh-key",
            "proxied-request",
            "one/same",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(output.content, "你好");
    }
    relay.abort();
    blocker.abort();
    assert!(
        result.is_ok(),
        "HTTP succeeds through the proxy but WebSocket failed: {:?}",
        result.err()
    );
    assert!(proxy_requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.starts_with(&format!(
            "GET http://localhost:{blocked_port}/api/remote.mux "
        ))));
}

#[tokio::test]
async fn model_catalog_and_websocket_share_client_dns_resolution() {
    let gateway = mock_gateway(false, false).await;
    let addr = url::Url::parse(&gateway.base)
        .unwrap()
        .socket_addrs(|| None)
        .unwrap()[0];
    let client = reqwest::Client::builder()
        .http1_only()
        .no_proxy()
        .resolve("dsh-resolution.invalid", addr)
        .build()
        .unwrap();
    let service = DshService::with_client(client);
    let config = provider(format!("http://dsh-resolution.invalid:{}", addr.port()));
    assert_eq!(
        service
            .fetch_models(&config, "test-dsh-key")
            .await
            .unwrap()
            .len(),
        3
    );
    let output = run_turn(
        &service,
        &config,
        "test-dsh-key",
        "dns-request",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.content, "你好");
}

#[tokio::test]
async fn upgraded_socket_remains_usable_after_the_http_request_timeout() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let mux = service
        .gateway(&config, "test-dsh-key")
        .unwrap()
        .mux()
        .await
        .unwrap()
        .0;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    tokio::time::resume();
    assert!(!mux.is_closed());
    let output = run_turn(
        &service,
        &config,
        "test-dsh-key",
        "long-lived-socket",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.content, "你好");
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn non_upgrade_and_malformed_handshake_responses_report_specific_errors() {
    for (response, code, status, hint) in [
        ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", "DSH_WEBSOCKET_HTTP_ERROR", Some(200), "普通网页"),
        ("HTTP/1.1 302 Found\r\nLocation: http://untrusted.invalid/\r\nContent-Length: 0\r\n\r\n", "DSH_WEBSOCKET_HTTP_ERROR", Some(302), "重定向"),
        ("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n", "DSH_WEBSOCKET_PROTOCOL_ERROR", None, "Connection: Upgrade"),
        ("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid\r\n\r\n", "DSH_WEBSOCKET_PROTOCOL_ERROR", None, "Sec-WebSocket-Accept"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = provider(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let service = DshService::new().unwrap();
        let gateway = service.gateway(&config, "test-dsh-key").unwrap();
        let error = gateway.mux().await.err().unwrap();
        assert_eq!(error.code, code);
        assert_eq!(error.status, status);
        assert!(error.message.contains(hint));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn refused_websocket_connection_reports_network_error_instead_of_proxy_support() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = provider(format!("http://{}", listener.local_addr().unwrap()));
    drop(listener);
    let service = DshService::new().unwrap();
    let gateway = service.gateway(&config, "test-dsh-key").unwrap();
    let error = gateway.mux().await.err().unwrap();
    assert_eq!(error.code, "DSH_WEBSOCKET_NETWORK_ERROR");
    assert!(error.message.contains("ConnectionRefused"));
    assert!(!error.message.contains("test-dsh-key"));
}

#[tokio::test]
async fn warm_turns_reuse_cookie_workspace_and_socket_but_have_independent_sessions_and_models() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    service.fetch_models(&config, "").await.unwrap();
    let first = run_turn(
        &service,
        &config,
        "",
        "request-one",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let second = run_turn(
        &service,
        &config,
        "",
        "request-two",
        "two/org/model",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(first.content, "你好");
    assert_eq!(second.content, "第二轮");
    wait_for_calls(&gateway, "$events/result", 4).await;
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "workspace/create")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/create")
            .count(),
        2
    );
    let selections: Vec<_> = calls
        .iter()
        .filter(|call| call["method"] == "session/selectModel")
        .map(|call| &call["payload"]["args"]["request"])
        .collect();
    assert_ne!(selections[0]["sessionId"], selections[1]["sessionId"]);
    assert_eq!(selections[0]["provider"], "one");
    assert_eq!(selections[1]["provider"], "two");
    assert_eq!(selections[1]["model"], "org/model");
}

#[tokio::test]
async fn completed_stream_does_not_wait_for_a_delayed_prompt_acknowledgement() {
    let gateway = mock_gateway_options(false, false, false, true).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "slow-ack",
            "one/same",
            &CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    // The HTTP response is still gated. Completion was proved by the correlated
    // durable user/message and turn/end; no artificial timing threshold is used.
    assert_eq!(gateway.ack_gate.available_permits(), 0);
    assert_eq!(result.content, "你好");
    assert_eq!(
        gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        1
    );
    gateway.ack_gate.add_permits(1);
}

#[tokio::test]
async fn concurrent_turns_share_a_socket_without_mixing_their_answers() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let cancellation = CancellationToken::new();
    let (first, second) = tokio::join!(
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "parallel-one",
            "one/same",
            &cancellation
        ),
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "parallel-two",
            "two/org/model",
            &cancellation
        )
    );
    let mut texts = vec![first.unwrap().content, second.unwrap().content];
    texts.sort();
    let mut expected = vec!["你好".to_owned(), "第二轮".to_owned()];
    expected.sort();
    assert_eq!(texts, expected);
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
    let commands = gateway.commands.lock().unwrap();
    let follows: Vec<_> = commands
        .iter()
        .filter(|command| command["endpoint"] == "session/follow")
        .collect();
    assert_eq!(follows.len(), 2);
    assert_ne!(follows[0]["streamId"], follows[1]["streamId"]);
}

#[tokio::test]
async fn cached_cookie_reauthenticates_once_after_an_authentication_rejection() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    service.fetch_models(&config, "").await.unwrap();
    gateway.reject_auth.store(true, Ordering::SeqCst);
    assert_eq!(service.fetch_models(&config, "").await.unwrap().len(), 3);
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/modelCatalog")
            .count(),
        3
    );
    assert!(!calls.iter().any(|call| call["method"] == "session/prompt"));
}

#[tokio::test]
async fn changing_credentials_or_endpoint_creates_an_isolated_gateway() {
    use std::sync::atomic::Ordering;
    let first_gateway = mock_gateway(false, false).await;
    let second_gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let mut config = provider(first_gateway.base.clone());
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "initial",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    run_turn(
        &service,
        &config,
        "rotated-key",
        "rotated",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(first_gateway.sockets.load(Ordering::SeqCst), 2);
    config.base_url = second_gateway.base.clone();
    run_turn(
        &service,
        &config,
        "rotated-key",
        "new-endpoint",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(second_gateway.sockets.load(Ordering::SeqCst), 1);
    assert_eq!(
        second_gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "workspace/create")
            .count(),
        1
    );
}

#[tokio::test]
async fn disconnect_does_not_replay_a_prompt_and_the_next_request_reconnects() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(true, false).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let cancellation = CancellationToken::new();
    let disconnect = async {
        wait_for_calls(&gateway, "session/prompt", 1).await;
        gateway.signals.send(MockSignal::Disconnect).unwrap();
    };
    let (failed, ()) = tokio::join!(
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "held-request",
            "one/same",
            &cancellation
        ),
        disconnect
    );
    assert_eq!(failed.unwrap_err().code, "DSH_DISCONNECTED");
    assert_eq!(
        gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        1
    );
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "unblocked-request",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 2);
    assert_eq!(
        gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        2
    );
}

#[tokio::test]
async fn cancelling_one_subscription_keeps_another_turn_running() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(true, false).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    let cancelled = CancellationToken::new();
    let other = CancellationToken::new();
    let stop = async {
        wait_for_calls(&gateway, "session/prompt", 2).await;
        cancelled.cancel();
    };
    let (stopped, completed, ()) = tokio::join!(
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "held-request",
            "one/same",
            &cancelled
        ),
        run_turn(
            &service,
            &config,
            "test-dsh-key",
            "unblocked-request",
            "two/org/model",
            &other
        ),
        stop
    );
    assert_eq!(stopped.unwrap_err().code, "CANCELLED");
    assert!(!completed.unwrap().content.is_empty());
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
    let calls = gateway.calls.lock().unwrap();
    let held = calls
        .iter()
        .find(|call| {
            call["method"] == "session/prompt"
                && call["payload"]["args"]["request"]["requestId"] == "held-request"
        })
        .unwrap();
    let cancel = calls
        .iter()
        .find(|call| call["method"] == "session/cancel")
        .unwrap();
    assert_eq!(
        held["payload"]["args"]["request"]["sessionId"],
        cancel["payload"]["args"]["request"]["sessionId"]
    );
}

#[tokio::test]
async fn idle_socket_is_released_and_a_later_request_uses_a_new_connection() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "before-idle",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let cached = service.gateway(&config, "test-dsh-key").unwrap();
    let (mux, reused, _) = cached.mux().await.unwrap();
    assert!(reused);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if gateway
                .commands
                .lock()
                .unwrap()
                .iter()
                .any(|command| command["type"] == "cancel")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    for _ in 0..100 {
        if mux.is_closed() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let closed = mux.is_closed();
    tokio::time::resume();
    assert!(closed);
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "after-idle",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn changed_workspace_has_a_separate_cache_entry_on_the_same_connection() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let mut config = provider(gateway.base.clone());
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "workspace-one",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    config.dsh_workspace = Some("/project".to_owned());
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "workspace-two",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    run_turn(
        &service,
        &config,
        "test-dsh-key",
        "workspace-two-again",
        "one/same",
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(gateway.sockets.load(Ordering::SeqCst), 1);
    let calls = gateway.calls.lock().unwrap();
    let paths: Vec<_> = calls
        .iter()
        .filter(|call| call["method"] == "workspace/create")
        .map(|call| call["payload"]["args"]["request"]["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["/workspace", "/project"]);
    let sessions: Vec<_> = calls
        .iter()
        .filter(|call| call["method"] == "session/create")
        .map(|call| &call["payload"]["args"]["request"]["workspaceId"])
        .collect();
    assert_ne!(sessions[0], sessions[1]);
    assert_eq!(sessions[1], sessions[2]);
}

#[tokio::test]
async fn concurrent_catalog_requests_recover_from_an_expired_cookie() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    let service = DshService::new().unwrap();
    let config = provider(gateway.base.clone());
    service.fetch_models(&config, "").await.unwrap();
    gateway.reject_auth.store(true, Ordering::SeqCst);
    let (first, second) = tokio::join!(
        service.fetch_models(&config, ""),
        service.fetch_models(&config, "")
    );
    assert_eq!(first.unwrap().len(), 3);
    assert_eq!(second.unwrap().len(), 3);
    assert_eq!(
        gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        2
    );
}

fn test_file(name: &str, mime: &str, bytes: &[u8]) -> crate::quick_ask::Attachment {
    use base64::Engine;
    crate::quick_ask::Attachment {
        name: name.into(),
        mime: mime.into(),
        kind: "file".into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

#[tokio::test]
async fn attachments_upload_binary_files_and_submit_receipts_in_the_same_session() {
    for cookie_login in [false, true] {
        let gateway = mock_gateway_auth(false, false, cookie_login).await;
        let key = if cookie_login { "" } else { "test-dsh-key" };
        let attachments = vec![
            crate::quick_ask::Attachment {
                name: "notes.txt".into(),
                mime: "text/plain".into(),
                kind: "text".into(),
                data: "实际文件内容\n第二行".into(),
            },
            test_file(
                "报告 & notes+#.pdf",
                "application/pdf",
                b"%PDF-1.7\nexample",
            ),
            test_file(
                "report.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                b"PK\x03\x04\x00example",
            ),
        ];
        let result = DshService::new()
            .unwrap()
            .generate_with_attachments(
                "files-request",
                &provider(gateway.base.clone()),
                key,
                "one/same",
                &plan(),
                &json!([{"role":"user","content":"分析附件"}]),
                &attachments,
                &CancellationToken::new(),
                &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
                |_| ControlFlow::Continue(()),
                |_| ControlFlow::Continue(()),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(result.content, "你好");
        let uploads = gateway.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 3);
        assert_eq!(uploads[0].bytes, attachments[0].data.as_bytes());
        assert_eq!(uploads[1].name, attachments[1].name);
        assert_eq!(uploads[1].bytes, b"%PDF-1.7\nexample");
        assert_eq!(uploads[2].bytes, b"PK\x03\x04\x00example");
        let calls = gateway.calls.lock().unwrap();
        let prompt = calls
            .iter()
            .find(|call| call["method"] == "session/prompt")
            .unwrap();
        let request = &prompt["payload"]["args"]["request"];
        for (index, upload) in uploads.iter().enumerate() {
            assert_eq!(upload.session_id, request["sessionId"].as_str().unwrap());
            assert_eq!(
                request["content"][index + 1],
                json!({"type":"file", "receiptId":upload.receipt})
            );
        }
        assert!(!request["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(&attachments[0].data));
        let first_upload = calls
            .iter()
            .position(|call| call["method"] == "session/uploadFileBinary")
            .unwrap();
        let submit = calls
            .iter()
            .position(|call| call["method"] == "session/prompt")
            .unwrap();
        assert!(first_upload < submit);
    }
}

#[tokio::test]
async fn images_use_native_media_parts_and_do_not_upload_as_generic_files() {
    use base64::Engine;
    let gateway = mock_gateway(false, false).await;
    let bytes = include_bytes!("../../assets/popper.png");
    let image = crate::quick_ask::Attachment {
        name: "photo.png".into(),
        mime: "image/png".into(),
        kind: "image".into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    };
    DshService::new()
        .unwrap()
        .generate_with_attachments(
            "image-request",
            &provider(gateway.base.clone()),
            "test-dsh-key",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"描述这张图片"}]),
            &[image.clone()],
            &CancellationToken::new(),
            &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap();
    assert!(gateway.uploads.lock().unwrap().is_empty());
    let calls = gateway.calls.lock().unwrap();
    let prompt = calls
        .iter()
        .find(|call| call["method"] == "session/prompt")
        .unwrap();
    assert_eq!(
        prompt["payload"]["args"]["request"]["content"][1],
        json!({
            "type":"image", "mediaType":"image/png", "data":image.data, "name":"photo.png"
        })
    );
}

#[tokio::test]
async fn attachment_upload_business_errors_and_invalid_receipts_never_submit_prompt() {
    for (body, expected) in [
        (
            json!({"ok":false,"error":{"message":"bad test-dsh-key"}}).to_string(),
            "DSH_ATTACHMENT_UPLOAD_REJECTED",
        ),
        (
            json!({"ok":true,"value":{}}).to_string(),
            "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
        ),
        (
            json!({"ok":true,"value":{"receiptId":17}}).to_string(),
            "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
        ),
        (
            json!({"value":{"receiptId":"receipt"}}).to_string(),
            "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
        ),
        ("not-json".into(), "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE"),
    ] {
        let gateway = mock_gateway(false, false).await;
        *gateway.upload_response.lock().unwrap() = Some(body);
        let error = DshService::new()
            .unwrap()
            .generate_with_attachments(
                "invalid-upload",
                &provider(gateway.base.clone()),
                "test-dsh-key",
                "one/same",
                &plan(),
                &json!([{"role":"user","content":"读取文档"}]),
                &[test_file("test.pdf", "application/pdf", b"%PDF-test")],
                &CancellationToken::new(),
                &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
                |_| ControlFlow::Continue(()),
                |_| ControlFlow::Continue(()),
                |_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.message.contains("test-dsh-key"));
        let calls = gateway.calls.lock().unwrap();
        assert!(!calls.iter().any(|call| call["method"] == "session/prompt"));
        assert!(calls.iter().any(|call| call["method"] == "session/cancel"));
    }
}

#[tokio::test]
async fn attachment_upload_http_failures_do_not_replay_or_submit_a_prompt() {
    use std::sync::atomic::Ordering;
    for status in [401, 404, 413, 503] {
        let gateway = mock_gateway(false, false).await;
        gateway.upload_status.store(status, Ordering::SeqCst);
        let error = DshService::new()
            .unwrap()
            .generate_with_attachments(
                "failed-upload",
                &provider(gateway.base.clone()),
                "test-dsh-key",
                "one/same",
                &plan(),
                &json!([{"role":"user","content":"读取文档"}]),
                &[test_file("test.pdf", "application/pdf", b"%PDF-test")],
                &CancellationToken::new(),
                &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
                |_| ControlFlow::Continue(()),
                |_| ControlFlow::Continue(()),
                |_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "DSH_ATTACHMENT_UPLOAD_HTTP_ERROR");
        assert_eq!(error.status, Some(status));
        assert_eq!(error.retryable, status >= 500);
        assert_eq!(gateway.uploads.lock().unwrap().len(), 1);
        assert!(!gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["method"] == "session/prompt"));
    }
}

#[tokio::test]
async fn rejected_upload_cookie_reauthenticates_before_receipt_admission() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway_auth(false, false, true).await;
    gateway.upload_status.store(401, Ordering::SeqCst);
    DshService::new()
        .unwrap()
        .generate_with_attachments(
            "cookie-upload",
            &provider(gateway.base.clone()),
            "",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"读取文档"}]),
            &[test_file("test.pdf", "application/pdf", b"%PDF-test")],
            &CancellationToken::new(),
            &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(gateway.uploads.lock().unwrap().len(), 2);
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "local-login")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        1
    );
}

#[tokio::test]
async fn later_generations_reupload_retained_attachments_into_their_new_sessions() {
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let attachment = test_file("test.pdf", "application/pdf", b"%PDF-test");
    for request_id in ["first-upload", "retry-upload", "follow-up-upload"] {
        service.generate_with_attachments(
            request_id, &provider(gateway.base.clone()), "test-dsh-key", "one/same", &plan(),
            &json!([{"role":"user","content":"读取文档"},{"role":"assistant","content":"原回答"},{"role":"user","content":"继续分析"}]), &[attachment.clone()],
            &CancellationToken::new(), &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()), |_| ControlFlow::Continue(()), |_| {},
        ).await.unwrap();
    }
    let uploads = gateway.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 3);
    assert_ne!(uploads[0].session_id, uploads[1].session_id);
    assert_ne!(uploads[1].session_id, uploads[2].session_id);
    let calls = gateway.calls.lock().unwrap();
    for prompt in calls
        .iter()
        .filter(|call| call["method"] == "session/prompt")
    {
        let request = &prompt["payload"]["args"]["request"];
        let upload = uploads
            .iter()
            .find(|upload| upload.session_id == request["sessionId"].as_str().unwrap())
            .unwrap();
        assert_eq!(request["content"][1]["receiptId"], upload.receipt);
    }
}

#[tokio::test]
async fn cancellation_during_upload_cancels_the_session_without_submitting_prompt() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    gateway.upload_blocked.store(true, Ordering::SeqCst);
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let seen = gateway.upload_seen.clone();
    let cancel_task = tokio::spawn(async move {
        let _permit = seen.acquire().await.unwrap();
        trigger.cancel();
    });
    let error = DshService::new()
        .unwrap()
        .generate_with_attachments(
            "cancel-upload",
            &provider(gateway.base.clone()),
            "test-dsh-key",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"读取文档"}]),
            &[test_file("test.pdf", "application/pdf", b"%PDF-test")],
            &cancellation,
            &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap_err();
    cancel_task.await.unwrap();
    assert_eq!(error.code, "CANCELLED");
    let calls = gateway.calls.lock().unwrap();
    assert!(!calls.iter().any(|call| call["method"] == "session/prompt"));
    let cancel = calls
        .iter()
        .find(|call| call["method"] == "session/cancel")
        .unwrap();
    assert_eq!(
        cancel["payload"]["args"]["request"]["sessionId"],
        gateway.uploads.lock().unwrap()[0].session_id
    );
    gateway.upload_gate.add_permits(1);
}

#[tokio::test]
async fn generation_budget_covers_the_attachment_upload_phase() {
    use std::sync::atomic::Ordering;
    let gateway = mock_gateway(false, false).await;
    gateway.upload_blocked.store(true, Ordering::SeqCst);
    let error = DshService::new()
        .unwrap()
        .generate_with_attachments(
            "timeout-upload",
            &provider(gateway.base.clone()),
            "test-dsh-key",
            "one/same",
            &plan(),
            &json!([{"role":"user","content":"读取文档"}]),
            &[test_file("test.pdf", "application/pdf", b"%PDF-test")],
            &CancellationToken::new(),
            &GenerationBudget::new(Instant::now(), Duration::from_secs(2)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "TOTAL_TIMEOUT");
    assert!(error.retryable);
    assert_eq!(gateway.uploads.lock().unwrap().len(), 1);
    assert!(!gateway
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call["method"] == "session/prompt"));
    gateway.upload_gate.add_permits(1);
}

#[tokio::test]
async fn upload_disconnect_allows_manual_retry_in_a_new_session_without_automatic_replay() {
    for response in ["disconnect-before-headers", "disconnect-during-body"] {
        let gateway = mock_gateway(false, false).await;
        *gateway.upload_response.lock().unwrap() = Some(response.into());
        let service = DshService::new().unwrap();
        let attachment = test_file("test.pdf", "application/pdf", b"%PDF-test");
        let error = service
            .generate_with_attachments(
                "disconnect-upload",
                &provider(gateway.base.clone()),
                "test-dsh-key",
                "one/same",
                &plan(),
                &json!([{"role":"user","content":"读取文档"}]),
                &[attachment.clone()],
                &CancellationToken::new(),
                &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
                |_| ControlFlow::Continue(()),
                |_| ControlFlow::Continue(()),
                |_| {},
            )
            .await
            .unwrap_err();
        assert!(error.retryable);
        assert_eq!(gateway.uploads.lock().unwrap().len(), 1);
        assert!(!gateway
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["method"] == "session/prompt"));
        *gateway.upload_response.lock().unwrap() = None;
        service
            .generate_with_attachments(
                "manual-retry-upload",
                &provider(gateway.base.clone()),
                "test-dsh-key",
                "one/same",
                &plan(),
                &json!([{"role":"user","content":"读取文档"}]),
                &[attachment],
                &CancellationToken::new(),
                &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
                |_| ControlFlow::Continue(()),
                |_| ControlFlow::Continue(()),
                |_| {},
            )
            .await
            .unwrap();
        let uploads = gateway.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 2);
        assert_ne!(uploads[0].session_id, uploads[1].session_id);
        assert_eq!(
            gateway
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call["method"] == "session/prompt")
                .count(),
            1
        );
    }
}

async fn conversation_turn(
    service: &DshService,
    conversation: &DshConversation,
    config: &ProviderConfig,
    key: &str,
    request_id: &str,
    model: &str,
    messages: &Value,
    all: &[crate::quick_ask::Attachment],
    latest: &[crate::quick_ask::Attachment],
    cancellation: &CancellationToken,
) -> Result<TransportSuccess, TransportFailure> {
    service
        .generate_in_conversation(
            conversation,
            request_id,
            config,
            key,
            model,
            &plan(),
            messages,
            all,
            latest,
            cancellation,
            &GenerationBudget::new(Instant::now(), Duration::from_secs(5)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
}

#[tokio::test]
async fn save_promotes_the_bound_session_and_continues_without_replaying_history_or_files() {
    for key in ["", "test-dsh-key"] {
        let gateway = mock_gateway_auth(false, false, key.is_empty()).await;
        let config = provider(gateway.base.clone());
        let service = DshService::new().unwrap();
        let conversation = DshConversation::default();
        let file = test_file("keep.txt", "text/plain", b"preserve this file");
        conversation_turn(&service, &conversation, &config, key, "before-save", "one/same",
            &json!([{"role":"user","content":"Original question"}]),
            &[file.clone()], &[file.clone()], &CancellationToken::new()).await.unwrap();
        let before = conversation.status(&config.id).await.unwrap();
        assert!(!before.saved);
        let saved = conversation.save(&config.id).await.unwrap();
        assert_eq!(saved.session_id, before.session_id);
        assert!(saved.saved);
        assert_eq!(conversation.save(&config.id).await.unwrap(), saved);
        let calls = gateway.calls.lock().unwrap().clone();
        let saves: Vec<_> = calls.iter().filter(|call| call["method"] == "temporary/save").collect();
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0]["payload"]["args"], json!({"request":{"sessionId":before.session_id}}));
        conversation_turn(&service, &conversation, &config, key, "after-save", "two/same",
            &json!([{"role":"user","content":"Original question"},
                {"role":"assistant","content":"Previous answer"},
                {"role":"user","content":"Continue saved conversation"}]),
            &[file.clone()], &[], &CancellationToken::new()).await.unwrap();
        assert_eq!(conversation.status(&config.id).await.unwrap(), saved);
        assert_eq!(gateway.sockets.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(gateway.uploads.lock().unwrap().len(), 1);
        let calls = gateway.calls.lock().unwrap();
        let creates: Vec<_> = calls.iter().filter(|call| call["method"] == "session/create").collect();
        assert_eq!(creates.len(), 2);
        assert_eq!(creates[1]["payload"]["args"], json!({"request":{"sessionId":saved.session_id}}));
        let prompt = calls.iter().rev().find(|call| call["method"] == "session/prompt").unwrap();
        assert_eq!(prompt["payload"]["args"]["request"]["sessionId"], saved.session_id);
        assert_eq!(prompt["payload"]["args"]["request"]["content"], json!([{"type":"text","text":"Continue saved conversation"}]));
    }
}

#[tokio::test]
async fn save_rejects_missing_foreign_or_busy_conversations_without_dispatch() {
    let conversation = DshConversation::default();
    assert_eq!(conversation.save("dsh").await.unwrap_err().code, "DSH_NO_SESSION");
    let _guard = conversation.state.lock().await;
    assert_eq!(conversation.save("dsh").await.unwrap_err().code, "DSH_SESSION_ACTIVE");
    drop(_guard);
    let gateway = mock_gateway(false, false).await;
    let config = provider(gateway.base.clone());
    let service = DshService::new().unwrap();
    conversation_turn(&service, &conversation, &config, "test-dsh-key", "first", "one/same",
        &json!([{"role":"user","content":"hello"}]), &[], &[], &CancellationToken::new()).await.unwrap();
    assert!(conversation.status("foreign-provider").await.is_none());
    assert_eq!(conversation.save("foreign-provider").await.unwrap_err().code, "DSH_NO_SESSION");
    assert!(!gateway.calls.lock().unwrap().iter().any(|call| call["method"] == "temporary/save"));
}

#[tokio::test]
async fn save_reports_plugin_errors_and_allows_idempotent_retry_after_an_uncertain_receipt() {
    let gateway = mock_gateway(false, false).await;
    let config = provider(gateway.base.clone());
    let service = DshService::new().unwrap();
    let conversation = DshConversation::default();
    conversation_turn(&service, &conversation, &config, "test-dsh-key", "first", "one/same",
        &json!([{"role":"user","content":"hello"}]), &[], &[], &CancellationToken::new()).await.unwrap();
    for (status, code, expected) in [
        (409, "temporary/session-active", "DSH_SESSION_ACTIVE"),
        (410, "temporary/expired", "DSH_SESSION_EXPIRED"),
        (404, "temporary/not-found", "DSH_NO_SESSION"),
        (503, "temporary/not-ready", "DSH_NOT_READY"),
        (404, "unknown", "DSH_SAVE_FAILED"),
        (500, "temporary/import-conflict", "DSH_SAVE_FAILED"),
        (200, "temporary/session-active", "DSH_SESSION_ACTIVE"),
    ] {
        *gateway.save_response.lock().unwrap() = Some((status, json!({"ok":false,
            "error":{"code":code,"message":"secret test-dsh-key internal path"}})));
        let error = conversation.save(&config.id).await.unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.message.contains("test-dsh-key"));
        assert!(!conversation.status(&config.id).await.unwrap().saved);
    }
    // No RPC failure envelope, as with an older gateway's missing route.
    *gateway.save_response.lock().unwrap() = Some((404, json!({})));
    assert_eq!(conversation.save(&config.id).await.unwrap_err().code, "DSH_SAVE_UNSUPPORTED");
    *gateway.save_response.lock().unwrap() = Some((200, json!({"ok":true,"value":{
        "sessionId":"wrong-session", "workspaceId":"normal", "savedSessionIds":["wrong-session"],
        "temporary":false, "reconnectRequired":true,
    }})));
    assert_eq!(conversation.save(&config.id).await.unwrap_err().code, "DSH_INVALID_RESPONSE");
    *gateway.save_response.lock().unwrap() = Some((200, json!({"ok":true,"value":{
        "sessionId":"remote-session", "workspaceId":"normal", "savedSessionIds":["remote-session"],
        "temporary":false, "reconnectRequired":true, "alreadySaved":true,
    }})));
    assert!(conversation.save(&config.id).await.unwrap().saved);
}

#[tokio::test]
async fn saving_one_window_keeps_another_windows_stream_usable() {
    let gateway = mock_gateway(false, false).await;
    let config = provider(gateway.base.clone());
    let service = DshService::new().unwrap();
    let first = DshConversation::default();
    let second = DshConversation::default();
    for (conversation, request) in [(&first, "one"), (&second, "two")] {
        conversation_turn(&service, conversation, &config, "test-dsh-key", request, "one/same",
            &json!([{"role":"user","content":"hello"}]), &[], &[], &CancellationToken::new()).await.unwrap();
    }
    let original_mux = service.gateway(&config, "test-dsh-key").unwrap().mux().await.unwrap().0;
    first.save(&config.id).await.unwrap();
    assert!(!original_mux.is_closed());
    conversation_turn(&service, &second, &config, "test-dsh-key", "two-next", "one/same",
        &json!([{"role":"user","content":"next"}]), &[], &[], &CancellationToken::new()).await.unwrap();
    assert_eq!(gateway.sockets.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!second.status(&config.id).await.unwrap().saved);
}

#[tokio::test]
async fn conversation_reuses_native_session_and_sends_only_new_questions_and_attachments() {
    for key in ["", "test-dsh-key"] {
        let gateway = mock_gateway_auth(false, false, key.is_empty()).await;
        let service = DshService::new().unwrap();
        let conversation = DshConversation::default();
        let config = provider(gateway.base.clone());
        let first_file = test_file("first.txt", "text/plain", b"first file");
        let next_file = test_file("next.txt", "text/plain", b"next file");
        let first = json!([
            {"role":"system","content":"Initial instructions and selected source"},
            {"role":"user","content":"First question"}
        ]);
        conversation_turn(
            &service,
            &conversation,
            &config,
            key,
            "first",
            "one/same",
            &first,
            &[first_file.clone()],
            &[first_file.clone()],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let second = json!([
            {"role":"system","content":"Initial instructions and selected source"},
            {"role":"user","content":"First question"},
            {"role":"assistant","content":"Previous answer"},
            {"role":"user","content":"Next question"}
        ]);
        // A cache expiration must not orphan the active conversation's Agent.
        service.clear_gateway_cache();
        conversation_turn(
            &service,
            &conversation,
            &config,
            key,
            "second",
            "two/org/model",
            &second,
            &[first_file.clone()],
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let third = json!([
            {"role":"system","content":"Answer questions instead of translating"},
            {"role":"user","content":"First question"},
            {"role":"assistant","content":"Previous answer"},
            {"role":"user","content":"Question with new file"}
        ]);
        conversation_turn(
            &service,
            &conversation,
            &config,
            key,
            "third",
            "two/org/model",
            &third,
            &[first_file.clone(), next_file.clone()],
            &[next_file],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        // User-triggered retry uses the same Agent, without replaying earlier turns.
        conversation_turn(
            &service,
            &conversation,
            &config,
            key,
            "retry",
            "one/same",
            &second,
            &[first_file],
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let calls = gateway.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call["method"] == "session/create")
                .count(),
            1
        );
        let prompts: Vec<_> = calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .map(|call| &call["payload"]["args"]["request"])
            .collect();
        assert_eq!(prompts.len(), 4);
        assert!(prompts
            .iter()
            .all(|prompt| prompt["sessionId"] == prompts[0]["sessionId"]));
        assert!(prompts[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("selected source"));
        assert_eq!(
            prompts[1]["content"],
            json!([{"type":"text","text":"Next question"}])
        );
        let updated = prompts[2]["content"][0]["text"].as_str().unwrap();
        assert!(updated.contains("Answer questions instead of translating"));
        assert!(updated.contains("Question with new file"));
        assert!(!updated.contains("First question"));
        assert!(!updated.contains("Previous answer"));
        assert_eq!(prompts[2]["content"].as_array().unwrap().len(), 2);
        assert!(!prompts[3]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Previous answer"));
        let selections: Vec<_> = calls
            .iter()
            .filter(|call| call["method"] == "session/selectModel")
            .map(|call| &call["payload"]["args"]["request"])
            .collect();
        assert!(selections
            .iter()
            .all(|selection| selection["sessionId"] == prompts[0]["sessionId"]));
        assert_eq!(selections[1]["provider"], "two");
        assert_eq!(selections[1]["model"], "org/model");
        let uploads = gateway.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].name, "first.txt");
        assert_eq!(uploads[1].name, "next.txt");
        assert_eq!(uploads[0].session_id, uploads[1].session_id);
    }
}

#[tokio::test]
async fn conversation_changes_destination_without_reusing_foreign_sessions_or_receipts() {
    let gateway = mock_gateway(false, false).await;
    let other_gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let conversation = DshConversation::default();
    let mut config = provider(gateway.base.clone());
    let messages = json!([
        {"role":"user","content":"Original question"},
        {"role":"assistant","content":"Original answer"},
        {"role":"user","content":"Latest question"}
    ]);
    let file = test_file("context.pdf", "application/pdf", b"%PDF-context");
    for (index, key) in ["test-dsh-key", "test-dsh-key", "rotated-key", "rotated-key"]
        .into_iter()
        .enumerate()
    {
        if index == 1 {
            config.dsh_workspace = Some("/other-workspace".into());
        }
        if index == 3 {
            config.id = "other-provider".into();
        }
        conversation_turn(
            &service,
            &conversation,
            &config,
            key,
            &format!("route-{index}"),
            "one/same",
            &messages,
            &[file.clone()],
            &[],
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    }
    config.base_url = other_gateway.base.clone();
    conversation_turn(
        &service,
        &conversation,
        &config,
        "test-dsh-key",
        "other-host",
        "one/same",
        &messages,
        &[file],
        &[],
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    for (server, expected) in [(&gateway, 4), (&other_gateway, 1)] {
        let calls = server.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call["method"] == "session/create")
                .count(),
            expected
        );
        assert_eq!(server.uploads.lock().unwrap().len(), expected);
        for call in calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
        {
            let text = call["payload"]["args"]["request"]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(text.contains("Original question"));
            assert!(text.contains("Original answer"));
            assert!(text.contains("Latest question"));
        }
    }
}

#[tokio::test]
async fn cancelled_conversation_retries_in_same_session_and_ignores_old_turn_events() {
    use std::sync::atomic::Ordering;
    // Receiving an HTTP request is not proof that the client has processed its
    // acknowledgement. Hold both the acknowledgement and output to make the
    // cancellation point deterministic, with and without existing context.
    for initialized in [false, true] {
        let gateway = mock_gateway_options(true, false, false, true).await;
        let service = Arc::new(DshService::new().unwrap());
        let conversation = Arc::new(DshConversation::default());
        let config = provider(gateway.base.clone());
        let messages = json!([
            {"role":"system","content":"Answer concisely"},
            {"role":"user","content":"Question"}
        ]);
        if initialized {
            // A correlated completed turn proves admission even while its HTTP
            // acknowledgement remains blocked by the same gate.
            conversation_turn(
                &service,
                &conversation,
                &config,
                "test-dsh-key",
                "unblocked-bootstrap",
                "one/same",
                &messages,
                &[],
                &[],
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        }
        let prior_prompts = usize::from(initialized);
        let cancellation = CancellationToken::new();
        let first = {
            let service = service.clone();
            let conversation = conversation.clone();
            let config = config.clone();
            let cancellation = cancellation.clone();
            let messages = messages.clone();
            tokio::spawn(async move {
                conversation_turn(
                    &service,
                    &conversation,
                    &config,
                    "test-dsh-key",
                    "blocked",
                    "one/same",
                    &messages,
                    &[],
                    &[],
                    &cancellation,
                )
                .await
            })
        };
        wait_for_calls(&gateway, "session/prompt", prior_prompts + 1).await;
        cancellation.cancel();
        assert_eq!(first.await.unwrap().unwrap_err().code, "CANCELLED");
        wait_for_calls(&gateway, "session/cancel", 1).await;
        {
            let remote = conversation.state.lock().await;
            let remote = remote.as_ref().unwrap();
            assert_eq!(remote.initialized, initialized);
            assert_eq!(
                remote.system.as_deref(),
                initialized.then_some("Answer concisely")
            );
        }
        gateway.stale_follow_events.store(true, Ordering::SeqCst);
        gateway.ack_gate.add_permits(3);
        let next = {
            let service = service.clone();
            let conversation = conversation.clone();
            let config = config.clone();
            tokio::spawn(async move {
                conversation_turn(
                    &service,
                    &conversation,
                    &config,
                    "test-dsh-key",
                    "unblocked-request",
                    "one/same",
                    &messages,
                    &[],
                    &[],
                    &CancellationToken::new(),
                )
                .await
            })
        };
        assert_eq!(next.await.unwrap().unwrap().content, "你好");
        let calls = gateway.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call["method"] == "session/create")
                .count(),
            1
        );
        let prompts: Vec<_> = calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .map(|call| &call["payload"]["args"]["request"])
            .collect();
        assert_eq!(prompts.len(), prior_prompts + 2);
        assert!(prompts
            .iter()
            .all(|prompt| prompt["sessionId"] == prompts[0]["sessionId"]));
        let bootstrap_prompt = prompts[0]["content"][0]["text"].as_str().unwrap();
        assert!(bootstrap_prompt.contains("Answer concisely"));
        assert!(bootstrap_prompt.contains("Question"));
        let expected_prompt = if initialized {
            "Question"
        } else {
            bootstrap_prompt
        };
        assert_eq!(
            prompts.last().unwrap()["content"][0]["text"],
            expected_prompt
        );
    }
}

#[tokio::test]
async fn waiting_for_conversation_lock_honors_cancellation_and_total_budget() {
    let service = DshService::new().unwrap();
    let conversation = DshConversation::default();
    let _lock = conversation.state.lock().await;
    let token = CancellationToken::new();
    token.cancel();
    let config = provider("http://127.0.0.1:1".into());
    let messages = json!([{"role":"user","content":"Question"}]);
    let error = conversation_turn(
        &service,
        &conversation,
        &config,
        "",
        "cancelled",
        "one/same",
        &messages,
        &[],
        &[],
        &token,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    let error = service
        .generate_in_conversation(
            &conversation,
            "timeout",
            &config,
            "",
            "one/same",
            &plan(),
            &messages,
            &[],
            &[],
            &CancellationToken::new(),
            &GenerationBudget::new(Instant::now(), Duration::from_millis(10)),
            |_| ControlFlow::Continue(()),
            |_| ControlFlow::Continue(()),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "TOTAL_TIMEOUT");
}

#[tokio::test]
async fn separate_windows_keep_independent_native_sessions_across_followups() {
    let gateway = mock_gateway(false, false).await;
    let service = DshService::new().unwrap();
    let first = DshConversation::default();
    let second = DshConversation::default();
    let config = provider(gateway.base.clone());
    let messages = json!([{"role":"user","content":"Initial question"}]);
    let token = CancellationToken::new();
    let (one, two) = tokio::join!(
        conversation_turn(
            &service,
            &first,
            &config,
            "test-dsh-key",
            "one",
            "one/same",
            &messages,
            &[],
            &[],
            &token
        ),
        conversation_turn(
            &service,
            &second,
            &config,
            "test-dsh-key",
            "two",
            "two/org/model",
            &messages,
            &[],
            &[],
            &token
        ),
    );
    let first_answer = one.unwrap().content;
    let second_answer = two.unwrap().content;
    assert_ne!(first_answer, second_answer);
    let followup = json!([
        {"role":"user","content":"Initial question"},
        {"role":"assistant","content":"Prior answer"},
        {"role":"user","content":"Follow-up"}
    ]);
    let (one, two) = tokio::join!(
        conversation_turn(
            &service,
            &first,
            &config,
            "test-dsh-key",
            "one-next",
            "one/same",
            &followup,
            &[],
            &[],
            &token
        ),
        conversation_turn(
            &service,
            &second,
            &config,
            "test-dsh-key",
            "two-next",
            "two/org/model",
            &followup,
            &[],
            &[],
            &token
        ),
    );
    assert_eq!(one.unwrap().content, first_answer);
    assert_eq!(two.unwrap().content, second_answer);
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/create")
            .count(),
        2
    );
    let id = |request_id: &str| {
        calls
            .iter()
            .find(|call| {
                call["method"] == "session/prompt"
                    && call["payload"]["args"]["request"]["requestId"] == request_id
            })
            .unwrap()["payload"]["args"]["request"]["sessionId"]
            .clone()
    };
    assert_eq!(id("one"), id("one-next"));
    assert_eq!(id("two"), id("two-next"));
    assert_ne!(id("one"), id("two"));
}

#[tokio::test]
async fn disconnected_conversation_reconnects_without_creating_or_replaying_a_session() {
    let gateway = mock_gateway(true, false).await;
    let service = DshService::new().unwrap();
    let conversation = DshConversation::default();
    let config = provider(gateway.base.clone());
    let messages = json!([{"role":"user","content":"Question"}]);
    let token = CancellationToken::new();
    let disconnect = async {
        wait_for_calls(&gateway, "session/prompt", 1).await;
        let _ = gateway.signals.send(MockSignal::Disconnect);
    };
    let (result, ()) = tokio::join!(
        conversation_turn(
            &service,
            &conversation,
            &config,
            "test-dsh-key",
            "blocked",
            "one/same",
            &messages,
            &[],
            &[],
            &token
        ),
        disconnect,
    );
    assert_eq!(result.unwrap_err().code, "DSH_DISCONNECTED");
    let result = conversation_turn(
        &service,
        &conversation,
        &config,
        "test-dsh-key",
        "unblocked-request",
        "one/same",
        &messages,
        &[],
        &[],
        &token,
    )
    .await
    .unwrap();
    assert_eq!(result.content, "你好");
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/create")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["method"] == "session/prompt")
            .count(),
        2
    );
}
