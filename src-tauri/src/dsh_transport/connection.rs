//! Reusable gateway resources. Remote Agent sessions are owned by Popper conversations.
use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use reqwest::{header, Client, Response, Upgraded};
use serde_json::{json, Value};
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex},
    time::Instant,
};
use tokio_tungstenite::{
    tungstenite::{
        error::{ProtocolError, SubProtocolError},
        handshake::{client::generate_key, derive_accept_key},
        protocol::{Role, WebSocketConfig},
        Error as WebSocketError, Message,
    },
    WebSocketStream,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{failure, invalid, safe_message, TransportFailure, RPC_LIMIT, RPC_TIMEOUT};
use crate::models::{ProviderConfig, MAX_PROVIDERS};

const CACHE_LIFETIME: Duration = Duration::from_secs(600);
const SOCKET_IDLE: Duration = Duration::from_secs(300);
const MAX_STREAMS: usize = 64;
const STREAM_QUEUE: usize = 128;
type Socket = WebSocketStream<Upgraded>;

pub(crate) struct DshService {
    client: Client,
    gateways: Mutex<HashMap<String, CacheEntry>>,
}
struct CacheEntry {
    gateway: Arc<Gateway>,
    used_at: Instant,
}
impl DshService {
    #[cfg(test)]
    pub(super) fn clear_gateway_cache(&self) {
        self.gateways.lock().clear();
    }

    pub(crate) fn new() -> Result<Self, TransportFailure> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .http1_only()
            .connect_timeout(RPC_TIMEOUT)
            .timeout(RPC_TIMEOUT)
            .pool_idle_timeout(SOCKET_IDLE)
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .user_agent(format!("Popper/{}", env!("POPPER_VERSION")))
            .build()
            .map_err(|_| failure("DSH_NETWORK_ERROR", "无法初始化 DSH 连接"))?;
        Ok(Self::with_client(client))
    }
    pub(super) fn with_client(client: Client) -> Self {
        Self {
            client,
            gateways: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn gateway(
        &self,
        provider: &ProviderConfig,
        key: &str,
    ) -> Result<Arc<Gateway>, TransportFailure> {
        let base = provider.base_url.trim_end_matches('/');
        if key.is_empty() && !crate::models::is_local_dsh_url(base) {
            return Err(failure(
                "DSH_API_KEY_MISSING",
                "远程 DSH 服务需要填写 DSH_API_KEY",
            ));
        }
        let mut gateways = self.gateways.lock();
        gateways.retain(|_, entry| entry.used_at.elapsed() < CACHE_LIFETIME);
        if let Some(entry) = gateways.get_mut(&provider.id) {
            if entry.gateway.rpc.base == base && entry.gateway.rpc.key == key {
                entry.used_at = Instant::now();
                return Ok(entry.gateway.clone());
            }
        }
        gateways.remove(&provider.id);
        if gateways.len() >= MAX_PROVIDERS {
            if let Some(oldest) = gateways
                .iter()
                .min_by_key(|(_, entry)| entry.used_at)
                .map(|(id, _)| id.clone())
            {
                gateways.remove(&oldest);
            }
        }
        let gateway = Arc::new(Gateway {
            rpc: Arc::new(Rpc {
                client: self.client.clone(),
                base: base.to_owned(),
                key: key.to_owned(),
                cookie: AsyncMutex::new(None),
            }),
            workspaces: AsyncMutex::new(HashMap::new()),
            mux: AsyncMutex::new(None),
        });
        gateways.insert(
            provider.id.clone(),
            CacheEntry {
                gateway: gateway.clone(),
                used_at: Instant::now(),
            },
        );
        Ok(gateway)
    }
}

pub(super) struct Gateway {
    pub rpc: Arc<Rpc>,
    workspaces: AsyncMutex<HashMap<String, String>>,
    mux: AsyncMutex<Option<Arc<Mux>>>,
}
impl Gateway {
    pub(super) fn after_promotion(&self) -> Arc<Self> {
        // Keep other result windows' active temporary streams independent.
        Arc::new(Self {
            rpc: self.rpc.clone(),
            workspaces: AsyncMutex::new(HashMap::new()),
            mux: AsyncMutex::new(None),
        })
    }
    pub(super) fn authentication_matches(&self, base: &str, key: &str) -> bool {
        self.rpc.base == base.trim_end_matches('/') && self.rpc.key == key
    }

    pub async fn check_stream(&self) -> Result<(), TransportFailure> {
        // Probe with a short-lived connection; do not create an Agent or retain
        // an event client that could receive unrelated GUI interactions.
        let _mux = Mux::connect(self.rpc.clone()).await?;
        Ok(())
    }
    pub async fn workspace(&self, path: &str) -> Result<(String, bool), TransportFailure> {
        let mut workspaces = self.workspaces.lock().await;
        if let Some(id) = workspaces.get(path) {
            return Ok((id.clone(), true));
        }
        let value = self
            .rpc
            .call("workspace/create", json!({"request":{"path":path}}))
            .await?;
        let id = value["workspace"]["workspaceId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?
            .to_owned();
        if workspaces.len() >= 8 {
            workspaces.clear();
        }
        workspaces.insert(path.to_owned(), id.clone());
        Ok((id, false))
    }
    pub async fn invalidate_workspaces(&self) {
        self.workspaces.lock().await.clear();
    }
    pub async fn mux(&self) -> Result<(Arc<Mux>, bool, Instant), TransportFailure> {
        let mut cached = self.mux.lock().await;
        if let Some(mux) = cached.as_ref().filter(|mux| !mux.is_closed()) {
            return Ok((mux.clone(), true, Instant::now()));
        }
        let mux = Mux::connect(self.rpc.clone()).await?;
        *cached = Some(mux.clone());
        Ok((mux, false, Instant::now()))
    }
}

pub(super) struct Rpc {
    client: Client,
    pub base: String,
    key: String,
    cookie: AsyncMutex<Option<(header::HeaderValue, Instant)>>,
}
impl Rpc {
    pub async fn authenticate(&self) -> Result<bool, TransportFailure> {
        if !self.key.is_empty() {
            return Ok(true);
        }
        let mut cached = self.cookie.lock().await;
        if cached
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < CACHE_LIFETIME)
        {
            return Ok(true);
        }
        let response = self
            .client
            .get(format!("{}/", self.base))
            .send()
            .await
            .map_err(|_| {
                failure(
                    "DSH_NETWORK_ERROR",
                    "无法连接本地 DSH，请检查网关地址与容器状态",
                )
            })?;
        if !matches!(response.status().as_u16(), 302 | 303) {
            return Err(failure("DSH_LOCAL_LOGIN_FAILED", "DSH 本地自动登录未成功，请确认 DSH_LOCAL_GUI_AUTO_LOGIN 已开启且网关使用本地发布地址，或填写 DSH_API_KEY"));
        }
        let cookies: Vec<&str> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.split(';').next())
            .filter(|value| {
                value
                    .split_once('=')
                    .is_some_and(|(name, value)| !name.is_empty() && !value.is_empty())
            })
            .collect();
        if cookies.is_empty() {
            return Err(failure(
                "DSH_LOCAL_LOGIN_FAILED",
                "DSH 本地自动登录未返回会话 Cookie，请检查自动登录配置或填写 DSH_API_KEY",
            ));
        }
        let mut value: header::HeaderValue = cookies.join("; ").parse().map_err(|_| invalid())?;
        value.set_sensitive(true);
        // Drain the redirect body to make its underlying HTTP connection reusable.
        read_body(response).await?;
        *cached = Some((value, Instant::now()));
        Ok(false)
    }
    async fn auth_headers(&self) -> Result<header::HeaderMap, TransportFailure> {
        let mut headers = header::HeaderMap::new();
        headers.insert("x-dsh-client", header::HeaderValue::from_static("app"));
        if self.key.is_empty() {
            loop {
                self.authenticate().await?;
                let cookie = self.cookie.lock().await;
                if let Some((value, _)) = cookie.as_ref() {
                    headers.insert(header::COOKIE, value.clone());
                    break;
                }
                // A concurrent 401 can invalidate the cache between these awaits.
            }
        } else {
            let mut value: header::HeaderValue = format!("Bearer {}", self.key)
                .parse()
                .map_err(|_| invalid())?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        Ok(headers)
    }
    async fn invalidate_cookie(&self, sent: Option<&header::HeaderValue>) {
        let mut cookie = self.cookie.lock().await;
        if cookie.as_ref().map(|(value, _)| value) == sent {
            *cookie = None;
        }
    }
    /// Binary uploads are a plain HTTP result, not a server-response RPC envelope.
    pub async fn upload_file(
        &self,
        session_id: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<String, TransportFailure> {
        let bytes = bytes::Bytes::from(bytes);
        let mut endpoint = url::Url::parse(&format!("{}/api/session/uploadFileBinary", self.base))
            .map_err(|_| invalid())?;
        endpoint
            .query_pairs_mut()
            .append_pair("sessionId", session_id)
            .append_pair("name", name);
        for attempt in 0..2 {
            let headers = self.auth_headers().await?;
            let response = self
                .client
                .post(endpoint.clone())
                .headers(headers.clone())
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .timeout(Duration::from_secs(60))
                .body(bytes.clone())
                .send()
                .await
                .map_err(|error| {
                    let mut failure = failure(
                        if error.is_timeout() {
                            "DSH_ATTACHMENT_UPLOAD_TIMEOUT"
                        } else {
                            "DSH_ATTACHMENT_UPLOAD_NETWORK_ERROR"
                        },
                        if error.is_timeout() {
                            "DSH 附件上传超时，请重试"
                        } else {
                            "DSH 附件上传失败，请检查网关连接"
                        },
                    );
                    failure.retryable = true;
                    failure
                })?;
            let status = response.status().as_u16();
            if status == 401 && self.key.is_empty() && attempt == 0 {
                // Authentication rejection precedes upload admission. A network
                // failure is never automatically retried, since it may be accepted.
                self.invalidate_cookie(headers.get(header::COOKIE)).await;
                continue;
            }
            if !response.status().is_success() {
                let message = match status {
                    401 | 403 => "DSH 附件上传认证失败，请检查 DSH_API_KEY 或本地自动登录".into(),
                    404 | 405 => "当前 DSH 后端未提供附件上传接口，请检查后端版本".into(),
                    413 => "DSH 拒绝附件大小，请缩小文件后重试".into(),
                    _ => format!("DSH 附件上传返回 HTTP {status}"),
                };
                let mut error = failure("DSH_ATTACHMENT_UPLOAD_HTTP_ERROR", message);
                error.retryable = status >= 500 || matches!(status, 408 | 429);
                error.status = Some(status);
                return Err(error);
            }
            let body = read_body(response).await.map_err(|mut error| {
                if error.code == "DSH_NETWORK_ERROR" {
                    error.retryable = true;
                }
                error
            })?;
            let envelope: Value = serde_json::from_slice(&body).map_err(|_| {
                failure(
                    "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
                    "DSH 附件上传响应无效",
                )
            })?;
            match envelope["ok"].as_bool() {
                Some(true) => {
                    let receipt = envelope["value"]["receiptId"]
                        .as_str()
                        .filter(|receipt| {
                            !receipt.trim().is_empty()
                                && receipt.len() <= 512
                                && !receipt.chars().any(char::is_control)
                        })
                        .ok_or_else(|| {
                            failure(
                                "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
                                "DSH 未返回有效的附件上传回执",
                            )
                        })?;
                    return Ok(receipt.to_owned());
                }
                Some(false) => {
                    return Err(failure(
                        "DSH_ATTACHMENT_UPLOAD_REJECTED",
                        format!(
                            "DSH 附件上传失败：{}",
                            safe_message(&envelope["error"], &self.key)
                        ),
                    ))
                }
                None => {
                    return Err(failure(
                        "DSH_ATTACHMENT_UPLOAD_INVALID_RESPONSE",
                        "DSH 附件上传响应缺少 ok 状态",
                    ))
                }
            }
        }
        Err(failure(
            "DSH_ATTACHMENT_UPLOAD_AUTH_ERROR",
            "DSH 附件上传认证失败",
        ))
    }

    pub async fn call(&self, method: &str, args: Value) -> Result<Value, TransportFailure> {
        let rpc_id = Uuid::new_v4().to_string();
        let body = json!({"type":"client-request", "rpcId":rpc_id, "method":method, "payload":{"args":args}});
        for attempt in 0..2 {
            let headers = self.auth_headers().await?;
            let request = self
                .client
                .post(format!("{}/api/{method}", self.base))
                .headers(headers.clone())
                .json(&body);
            let request = if method == "temporary/save" {
                request.timeout(Duration::from_secs(60))
            } else {
                request
            };
            let response = request
                .send()
                .await
                .map_err(|_| {
                    failure(
                        "DSH_NETWORK_ERROR",
                        "无法连接 DSH，请检查网关地址与容器状态",
                    )
                })?;
            let status = response.status().as_u16();
            if status == 401 && self.key.is_empty() && attempt == 0 {
                // A rejected authentication has not dispatched the RPC, so this retry cannot repeat a tool action.
                self.invalidate_cookie(headers.get(header::COOKIE)).await;
                continue;
            }
            if !response.status().is_success() {
                if method == "temporary/save" && !matches!(status, 401 | 403) {
                    let envelope = read_body(response).await.ok()
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                    let code = envelope.as_ref()
                        .filter(|value| value["type"] == "server-response" && value["rpcId"] == rpc_id && value["result"]["ok"] == false)
                        .and_then(|value| value["result"]["error"]["code"].as_str());
                    return Err(save_failure(code, Some(status)));
                }
                let mut error = failure(
                    "DSH_HTTP_ERROR",
                    match status {
                        401 | 403 if self.key.is_empty() => {
                            "DSH 本地会话认证失败，请检查自动登录配置或填写 DSH_API_KEY".to_owned()
                        }
                        401 | 403 => "DSH 认证失败，请检查 DSH_API_KEY 和网关权限".to_owned(),
                        503 => "DSH 尚未就绪，请等待容器启动完成".to_owned(),
                        _ => format!("DSH 返回 HTTP {status}"),
                    },
                );
                error.status = Some(status);
                return Err(error);
            }
            let envelope: Value =
                serde_json::from_slice(&read_body(response).await?).map_err(|_| invalid())?;
            if envelope["type"] != "server-response" || envelope["rpcId"] != rpc_id {
                return Err(invalid());
            }
            return match envelope["result"]["ok"].as_bool() {
                Some(true) => envelope["result"].get("value").cloned().ok_or_else(invalid),
                Some(false) if method == "temporary/save" => Err(save_failure(
                    envelope["result"]["error"]["code"].as_str(), Some(status),
                )),
                Some(false) => Err(failure(
                    "DSH_RPC_ERROR",
                    safe_message(&envelope["result"]["error"], &self.key),
                )),
                None => Err(invalid()),
            };
        }
        Err(invalid())
    }
}

fn save_failure(code: Option<&str>, status: Option<u16>) -> TransportFailure {
    let (code, message) = match (code, status) {
        (Some("temporary/session-active"), _) => ("DSH_SESSION_ACTIVE", "DSH 会话仍有生成、子任务或后台工作，请结束或取消后再保存"),
        (Some("temporary/expired"), _) | (_, Some(410)) => ("DSH_SESSION_EXPIRED", "DSH 临时会话已过期清理，无法保存"),
        (Some("temporary/not-found"), _) => ("DSH_NO_SESSION", "DSH 临时会话不存在，无法保存"),
        (Some("temporary/not-ready"), _) | (_, Some(503)) => ("DSH_NOT_READY", "DSH 临时会话服务尚未就绪，请稍后重试"),
        (None, Some(404)) => ("DSH_SAVE_UNSUPPORTED", "DSH 未提供保存接口，请安装并启用 dsh-temporary-sessions 插件"),
        _ => ("DSH_SAVE_FAILED", "保存 DSH 会话失败，请重试；保存接口支持重复提交"),
    };
    let mut error = failure(code, message);
    error.status = status;
    error
}
async fn read_body(response: Response) -> Result<Vec<u8>, TransportFailure> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| failure("DSH_NETWORK_ERROR", "DSH 响应读取失败"))?;
        if bytes.len().saturating_add(chunk.len()) > RPC_LIMIT {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) enum MuxItem {
    Value(Value),
    Notice(String),
    Failed,
}
struct Listener {
    session: String,
    sender: mpsc::Sender<MuxItem>,
}
enum Command {
    Open {
        id: String,
        session: String,
        sender: mpsc::Sender<MuxItem>,
    },
    Cancel(String),
    DelegateFailed(String),
}
pub(super) struct Mux {
    commands: mpsc::UnboundedSender<Command>,
    shutdown: CancellationToken,
}
impl Drop for Mux {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
pub(super) struct Subscription {
    id: String,
    commands: mpsc::UnboundedSender<Command>,
    pub receiver: mpsc::Receiver<MuxItem>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Cancel(self.id.clone()));
    }
}
impl Mux {
    pub fn is_closed(&self) -> bool {
        self.commands.is_closed()
    }
    async fn connect(rpc: Arc<Rpc>) -> Result<Arc<Self>, TransportFailure> {
        let endpoint = format!("{}/api/remote.mux", rpc.base);
        let mut socket = None;
        for attempt in 0..2 {
            let headers = rpc.auth_headers().await?;
            let key = generate_key();
            let deadline = Instant::now() + RPC_TIMEOUT;
            // Use the same DNS fallback, proxy, TLS and connection pool as HTTP
            // RPC. A separate direct TCP dial can stall while HTTP works.
            let response = tokio::time::timeout_at(
                deadline,
                rpc.client
                    .get(&endpoint)
                    .version(reqwest::Version::HTTP_11)
                    .headers(headers.clone())
                    .header(header::CONNECTION, "Upgrade")
                    .header(header::UPGRADE, "websocket")
                    .header(header::SEC_WEBSOCKET_VERSION, "13")
                    .header(header::SEC_WEBSOCKET_KEY, &key)
                    .send(),
            )
            .await
            .map_err(|_| websocket_timeout("建立 HTTP 连接并等待 Upgrade 响应"))?
            .map_err(websocket_request_failure)?;
            let status = response.status().as_u16();
            if status == 401 && rpc.key.is_empty() && attempt == 0 {
                rpc.invalidate_cookie(headers.get(header::COOKIE)).await;
                continue;
            }
            if status != 101 {
                return Err(websocket_http_failure(status, rpc.key.is_empty()));
            }
            validate_upgrade(response.headers(), &key)
                .map_err(|error| websocket_failure(error.into(), rpc.key.is_empty()))?;
            let upgraded = tokio::time::timeout_at(deadline, response.upgrade())
                .await
                .map_err(|_| websocket_timeout("接管已升级的连接"))?
                .map_err(websocket_request_failure)?;
            let config = WebSocketConfig::default()
                .max_message_size(Some(RPC_LIMIT))
                .max_frame_size(Some(RPC_LIMIT));
            socket =
                Some(WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await);
            break;
        }
        let mut socket = socket.ok_or_else(invalid)?;
        send(&mut socket, json!({"type":"open", "streamId":"events", "endpoint":"$events", "payload":{"args":{}}})).await?;
        let frame = tokio::time::timeout(RPC_TIMEOUT, receive(&mut socket))
            .await
            .map_err(|_| failure("DSH_STREAM_TIMEOUT", "DSH 事件订阅就绪超时"))??;
        if frame["type"] != "item"
            || frame["streamId"] != "events"
            || frame["value"]["type"] != "ready"
        {
            return Err(invalid());
        }
        let client_id = frame["value"]["clientId"]
            .as_str()
            .ok_or_else(invalid)?
            .to_owned();
        let (commands, receiver) = mpsc::unbounded_channel();
        let shutdown = CancellationToken::new();
        let mux = Arc::new(Self {
            commands: commands.clone(),
            shutdown: shutdown.clone(),
        });
        tokio::spawn(drive(socket, rpc, client_id, receiver, commands, shutdown));
        Ok(mux)
    }
    pub fn follow(&self, session: &str) -> Result<Subscription, TransportFailure> {
        let id = Uuid::new_v4().to_string();
        let (sender, receiver) = mpsc::channel(STREAM_QUEUE);
        self.commands
            .send(Command::Open {
                id: id.clone(),
                session: session.to_owned(),
                sender,
            })
            .map_err(|_| failure("DSH_DISCONNECTED", "DSH 连接已断开，请重试"))?;
        Ok(Subscription {
            id,
            commands: self.commands.clone(),
            receiver,
        })
    }
}

// Never format the full error: HTTP responses and IO errors may carry credentials,
// cookies, response bodies or proxy URLs. Preserve only safe diagnostic categories.
fn websocket_timeout(stage: &str) -> TransportFailure {
    failure(
        "DSH_WEBSOCKET_TIMEOUT",
        format!(
            "连接 DSH 会话流超时（15 秒）：{stage}未完成，请检查 /api/remote.mux 的网关及代理转发"
        ),
    )
}

fn websocket_request_failure(error: reqwest::Error) -> TransportFailure {
    if error.is_timeout() {
        return websocket_timeout(if error.is_connect() {
            "建立网络连接"
        } else {
            "等待 Upgrade 响应"
        });
    }
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(source) = cause {
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            let safe_io = io
                .raw_os_error()
                .map(std::io::Error::from_raw_os_error)
                .unwrap_or_else(|| std::io::Error::from(io.kind()));
            return websocket_failure(WebSocketError::Io(safe_io), false);
        }
        cause = source.source();
    }
    failure(
        "DSH_WEBSOCKET_NETWORK_ERROR",
        "DSH 会话流 HTTP Upgrade 请求失败，请检查网络、代理及 HTTPS 证书",
    )
}

fn validate_upgrade(headers: &header::HeaderMap, key: &str) -> Result<(), ProtocolError> {
    let has_token = |name: header::HeaderName, token: &str| {
        headers
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    };
    if !has_token(header::UPGRADE, "websocket") {
        return Err(ProtocolError::MissingUpgradeWebSocketHeader);
    }
    if !has_token(header::CONNECTION, "upgrade") {
        return Err(ProtocolError::MissingConnectionUpgradeHeader);
    }
    let expected = derive_accept_key(key.as_bytes());
    if headers
        .get(header::SEC_WEBSOCKET_ACCEPT)
        .map(|value| value.as_bytes())
        != Some(expected.as_bytes())
    {
        return Err(ProtocolError::SecWebSocketAcceptKeyMismatch);
    }
    if headers.contains_key(header::SEC_WEBSOCKET_PROTOCOL) {
        return Err(ProtocolError::SecWebSocketSubProtocolError(
            SubProtocolError::ServerSentSubProtocolNoneRequested,
        ));
    }
    if headers.contains_key(header::SEC_WEBSOCKET_EXTENSIONS) {
        return Err(ProtocolError::InvalidHeader(Box::new(
            header::SEC_WEBSOCKET_EXTENSIONS,
        )));
    }
    Ok(())
}

fn websocket_http_failure(status: u16, local_login: bool) -> TransportFailure {
    let hint = match status {
        401 if local_login => "本地登录 Cookie 被拒绝，请检查 DSH 自动登录配置及访问地址",
        401 => "网关拒绝认证，请检查 DSH_API_KEY",
        403 => "网关拒绝访问，请核对 DSH_PUBLIC_URL、可信 Host 及代理转发的 Origin",
        404 => "未找到 /api/remote.mux，请检查 API 地址及代理路径",
        200 => "网关返回了普通网页，请检查 /api/remote.mux 的 WebSocket Upgrade 转发",
        301 | 302 | 303 | 307 | 308 => "握手被重定向，请直接填写最终网关地址",
        502 | 503 | 504 => "网关或上游 DSH 尚不可用，请检查容器状态及代理上游",
        _ => "网关未接受 WebSocket 握手，请检查 /api/remote.mux 的转发",
    };
    let mut result = failure(
        "DSH_WEBSOCKET_HTTP_ERROR",
        format!("DSH 会话流握手失败（HTTP {status}）：{hint}"),
    );
    result.status = Some(status);
    result
}

fn websocket_failure(error: WebSocketError, local_login: bool) -> TransportFailure {
    match error {
        WebSocketError::Http(response) => {
            websocket_http_failure(response.status().as_u16(), local_login)
        }
        WebSocketError::Io(error) => {
            let kind = error.kind();
            let hint = match kind {
                std::io::ErrorKind::ConnectionRefused => "连接被拒绝，请检查网关端口及容器是否启动",
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof => {
                    "连接被对端关闭，请检查网关日志及 WebSocket 转发"
                }
                std::io::ErrorKind::TimedOut => "网络连接超时，请检查网关地址及端口",
                _ => "网络连接失败，请检查地址解析、网关端口及容器状态",
            };
            failure(
                "DSH_WEBSOCKET_NETWORK_ERROR",
                format!(
                    "无法连接 DSH 会话流：{hint}（{kind:?}，系统错误码：{}）",
                    error
                        .raw_os_error()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "无".into())
                ),
            )
        }
        WebSocketError::Tls(_) => failure(
            "DSH_WEBSOCKET_TLS_ERROR",
            "DSH 会话流 TLS 握手失败，请检查 HTTPS 证书及系统信任链",
        ),
        WebSocketError::Protocol(error) => {
            let detail = match error {
                ProtocolError::HandshakeIncomplete => "对端在握手完成前关闭了连接",
                ProtocolError::MissingConnectionUpgradeHeader => "响应缺少 Connection: Upgrade",
                ProtocolError::MissingUpgradeWebSocketHeader => "响应缺少 Upgrade: websocket",
                ProtocolError::SecWebSocketAcceptKeyMismatch => "Sec-WebSocket-Accept 无效",
                _ => "握手响应不符合 WebSocket 协议",
            };
            failure(
                "DSH_WEBSOCKET_PROTOCOL_ERROR",
                format!("DSH 会话流握手失败：{detail}，请检查网关日志及反向代理"),
            )
        }
        _ => failure(
            "DSH_WEBSOCKET_ERROR",
            "DSH 会话流握手失败，请检查网关日志及 WebSocket 转发",
        ),
    }
}
async fn send(socket: &mut Socket, value: Value) -> Result<(), TransportFailure> {
    tokio::time::timeout(
        RPC_TIMEOUT,
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| failure("DSH_STREAM_TIMEOUT", "发送 DSH 流请求超时"))?
    .map_err(|_| failure("DSH_DISCONNECTED", "DSH 连接已断开，请重试"))
}
async fn receive(socket: &mut Socket) -> Result<Value, TransportFailure> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|_| invalid())
            }
            Some(Ok(Message::Ping(bytes))) => {
                socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| invalid())?;
            }
            Some(Ok(Message::Pong(_))) => {}
            _ => return Err(failure("DSH_DISCONNECTED", "DSH 连接已断开，请重试")),
        }
    }
}
async fn drive(
    mut socket: Socket,
    rpc: Arc<Rpc>,
    client_id: String,
    mut commands: mpsc::UnboundedReceiver<Command>,
    command_sender: mpsc::UnboundedSender<Command>,
    shutdown: CancellationToken,
) {
    let mut listeners: HashMap<String, Listener> = HashMap::new();
    let idle = tokio::time::sleep(SOCKET_IDLE);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            _ = &mut idle, if listeners.is_empty() => break,
            command = commands.recv() => match command {
                Some(Command::Open { id, session, sender }) => {
                    if listeners.len() >= MAX_STREAMS { let _ = sender.try_send(MuxItem::Failed); continue; }
                    if send(&mut socket, json!({"type":"open", "streamId":id, "endpoint":"session/follow", "payload":{"args":{"request":{
                        "address":{"kind":"session", "sessionId":session}, "assistantStream":true}}}})).await.is_err() { break; }
                    listeners.insert(id, Listener { session, sender });
                }
                Some(Command::Cancel(id)) => {
                    if listeners.remove(&id).is_some() && send(&mut socket, json!({"type":"cancel", "streamId":id})).await.is_err() { break; }
                    if listeners.is_empty() { idle.as_mut().reset(Instant::now() + SOCKET_IDLE); }
                }
                Some(Command::DelegateFailed(id)) => {
                    if let Some(listener) = listeners.remove(&id) { let _ = listener.sender.try_send(MuxItem::Failed); }
                    if send(&mut socket, json!({"type":"cancel", "streamId":id})).await.is_err() { break; }
                    if listeners.is_empty() { idle.as_mut().reset(Instant::now() + SOCKET_IDLE); }
                }
                None => break,
            },
            frame = receive(&mut socket) => {
                let Ok(mut frame) = frame else { break; };
                let Some(stream_id) = frame["streamId"].as_str().map(str::to_owned) else { break; };
                if stream_id == "events" {
                    if frame["type"] != "item" { break; }
                    let value = &frame["value"];
                    match value["type"].as_str() {
                        Some("waterfall") => {
                            let owned = listeners.iter().find(|(_, listener)| value["agentId"].as_str() == Some(listener.session.as_str()));
                            if let Some((_, listener)) = owned {
                                if matches!(value["event"].as_str(), Some("approval/request" | "user-questions/request")) {
                                    let _ = listener.sender.try_send(MuxItem::Notice(format!("DSH 需要审批或回答问题，请打开 {} 并进入会话 {} 处理。", rpc.base, listener.session)));
                                }
                            }
                            let owner_id = owned.map(|(id, _)| id.clone());
                            let rpc = rpc.clone();
                            let sender = command_sender.clone();
                            let args = json!({"clientId":client_id, "eventId":value["eventId"], "outcome":{"kind":"next"}});
                            tokio::spawn(async move {
                                if rpc.call("$events/result", args).await.is_err() {
                                    if let Some(id) = owner_id { let _ = sender.send(Command::DelegateFailed(id)); }
                                }
                            });
                        }
                        Some("emit" | "cancel") => {}
                        _ => break,
                    }
                    continue;
                }
                let Some(listener) = listeners.get(&stream_id) else { continue; };
                let delivered = match frame["type"].as_str() {
                    Some("item") => listener.sender.try_send(MuxItem::Value(frame["value"].take())).is_ok(),
                    _ => { let _ = listener.sender.try_send(MuxItem::Failed); false },
                };
                if !delivered {
                    let id = stream_id.to_owned();
                    listeners.remove(&id);
                    if send(&mut socket, json!({"type":"cancel", "streamId":id})).await.is_err() { break; }
                    if listeners.is_empty() { idle.as_mut().reset(Instant::now() + SOCKET_IDLE); }
                }
            }
        }
    }
    commands.close();
    for listener in listeners.into_values() {
        let _ = listener.sender.try_send(MuxItem::Failed);
    }
    let _ = tokio::time::timeout(Duration::from_millis(500), socket.close(None)).await;
}
