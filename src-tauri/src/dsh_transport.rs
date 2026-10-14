//! DSH Docker gateway adapter, pinned to the native 0.2.0-rc.2 protocol.
//! Local clients use the gateway's automatic Cookie login; remote clients use a gateway key.
use std::{collections::HashSet, ops::ControlFlow, sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[path = "dsh_transport/attachments.rs"]
mod attachments;
#[path = "dsh_transport/connection.rs"]
mod connection;
pub(crate) use connection::DshService;
use connection::{Gateway, MuxItem};

use crate::{
    models::{
        ProviderConfig, ProviderModel, ThinkingCapability, ThinkingCapabilitySource,
        ThinkingDialect, ThinkingLevel, ThinkingMode, AI_OUTPUT_LIMIT,
    },
    openai_transport::{
        GenerationBudget, ResponseTermination, ResponseTransport, TransportFailure,
        TransportSuccess,
    },
    thinking::ThinkingRequestPlan,
};

const RPC_LIMIT: usize = 2 * 1024 * 1024;
const RPC_TIMEOUT: Duration = Duration::from_secs(15);

fn failure(code: &'static str, message: impl Into<String>) -> TransportFailure {
    TransportFailure {
        code,
        message: message.into(),
        retryable: false,
        status: None,
        provider_error: None,
        content_seen: false,
    }
}
fn invalid() -> TransportFailure {
    failure(
        "DSH_INVALID_RESPONSE",
        "DSH 响应格式无效，请核对后端版本（支持 0.2.0-rc.2）",
    )
}
fn cancelled() -> TransportFailure {
    failure("CANCELLED", "生成已停止")
}
fn safe_message(value: &Value, key: &str) -> String {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("DSH 请求失败");
    let message = if key.is_empty() {
        message.to_owned()
    } else {
        message.replace(key, "[redacted]")
    };
    message
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}

pub(crate) fn split_model_id(id: &str) -> Result<(&str, &str), TransportFailure> {
    id.split_once('/')
        .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
        .ok_or_else(|| {
            failure(
                "DSH_MODEL_INVALID",
                "DSH 模型 ID 格式应为 提供商ID/模型ID，请重新获取模型",
            )
        })
}

fn catalog_models(value: &Value) -> Result<Vec<ProviderModel>, TransportFailure> {
    let groups = value["groups"].as_array().ok_or_else(invalid)?;
    let mut models = Vec::new();
    let mut ids = HashSet::new();
    for group in groups {
        let provider = group["id"]
            .as_str()
            .filter(|id| !id.is_empty() && !id.contains('/'))
            .ok_or_else(invalid)?;
        for model in group["models"].as_array().ok_or_else(invalid)? {
            let model_id = model["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(invalid)?;
            let id = format!("{provider}/{model_id}");
            let name = format!(
                "{} · {}",
                group["name"].as_str().unwrap_or(provider),
                model["name"].as_str().unwrap_or(model_id)
            );
            if id.len() > 256 || name.len() > 256 || !ids.insert(id.clone()) {
                continue;
            }
            let efforts: Vec<&str> = model["reasoning"]["efforts"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["id"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            let levels = efforts
                .iter()
                .filter_map(|effort| match *effort {
                    "minimal" => Some(ThinkingLevel::Minimal),
                    "low" => Some(ThinkingLevel::Low),
                    "medium" => Some(ThinkingLevel::Medium),
                    "high" => Some(ThinkingLevel::High),
                    "xhigh" => Some(ThinkingLevel::Xhigh),
                    _ => None,
                })
                .collect();
            models.push(ProviderModel {
                id,
                name,
                thinking_levels: levels,
                thinking_capability: Some(ThinkingCapability {
                    source: ThinkingCapabilitySource::Explicit,
                    dialect: if efforts.is_empty() {
                        None
                    } else {
                        Some(ThinkingDialect::ReasoningEffort)
                    },
                    supports_off: efforts.contains(&"none"),
                }),
            });
            if models.len() == 200 {
                return Ok(models);
            }
        }
    }
    if models.is_empty() {
        return Err(failure(
            "DSH_NO_MODELS",
            "DSH 没有可用模型，请先在 DSH 网页的设置 → 模型中配置服务商",
        ));
    }
    Ok(models)
}

impl DshService {
    pub(crate) async fn check_connection(
        &self,
        provider: &ProviderConfig,
        key: &str,
    ) -> Result<Vec<ProviderModel>, TransportFailure> {
        let models = self.fetch_models(provider, key).await?;
        tokio::time::timeout(RPC_TIMEOUT, self.gateway(provider, key)?.check_stream())
            .await
            .map_err(|_| failure("DSH_WEBSOCKET_TIMEOUT", "DSH 会话流连接检查超时（15 秒）"))??;
        Ok(models)
    }
    pub(crate) async fn fetch_models(
        &self,
        provider: &ProviderConfig,
        key: &str,
    ) -> Result<Vec<ProviderModel>, TransportFailure> {
        let gateway = self.gateway(provider, key)?;
        let value = tokio::time::timeout(
            RPC_TIMEOUT,
            gateway.rpc.call("session/modelCatalog", json!({})),
        )
        .await
        .map_err(|_| failure("DSH_TIMEOUT", "获取 DSH 模型超时"))??;
        catalog_models(&value)
    }
}

fn prompt_text(messages: &Value) -> Result<String, TransportFailure> {
    // Bootstrap a new remote conversation with Popper's current context.
    let messages = messages.as_array().ok_or_else(invalid)?;
    let mut prompt =
        String::from("以下是 Popper 提交的对话。请根据对话中的任务要求回答最新用户消息。\n");
    for message in messages {
        let role = message["role"].as_str().ok_or_else(invalid)?;
        let content = message["content"].as_str().ok_or_else(invalid)?;
        prompt.push_str(&format!("\n<{role}>\n{content}\n</{role}>\n"));
    }
    Ok(prompt)
}

/// Owned by one Popper result session, independent of the gateway cache lifetime.
/// The turn lock prevents a retry from racing the preceding remote cancel.
#[derive(Default)]
pub(crate) struct DshConversation {
    state: tokio::sync::Mutex<Option<RemoteConversation>>,
}

struct RemoteConversation {
    gateway: Arc<Gateway>,
    provider_id: String,
    workspace: String,
    session_id: String,
    system: Option<String>,
    initialized: bool,
}

fn system_text(messages: &Value) -> Result<Option<String>, TransportFailure> {
    let messages = messages.as_array().ok_or_else(invalid)?;
    let systems = messages
        .iter()
        .filter(|message| message["role"] == "system")
        .map(|message| message["content"].as_str().ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((!systems.is_empty()).then(|| systems.join("\n\n")))
}

fn incremental_prompt(
    messages: &Value,
    previous_system: &Option<String>,
) -> Result<String, TransportFailure> {
    let latest = messages
        .as_array()
        .and_then(|messages| messages.last())
        .filter(|message| message["role"] == "user")
        .ok_or_else(invalid)?;
    let text = latest["content"].as_str().ok_or_else(invalid)?;
    let system = system_text(messages)?;
    if &system == previous_system {
        return Ok(text.to_owned());
    }
    // Update task instructions without replaying source text, answers or files.
    Ok(format!(
        "本轮任务指令（替换之前的任务指令）：\n<system>\n{}\n</system>\n\n<user>\n{text}\n</user>",
        system
            .as_deref()
            .unwrap_or("根据已有对话回答最新问题，不再执行之前的单次任务。")
    ))
}

#[derive(Default)]
struct Output {
    content: String,
    current: String,
    next_index: u64,
    attempt: Option<String>,
    scalars: usize,
}
impl Output {
    fn append<F: FnMut(String) -> ControlFlow<()>>(
        &mut self,
        text: &str,
        emit: &mut F,
    ) -> Result<(), TransportFailure> {
        self.scalars = self.scalars.saturating_add(text.chars().count());
        if self.scalars > AI_OUTPUT_LIMIT {
            return Err(failure("OUTPUT_LIMIT", "DSH 输出超过长度限制"));
        }
        if text.is_empty() {
            return Ok(());
        }
        if emit(text.to_owned()).is_break() {
            return Err(cancelled());
        }
        self.content.push_str(text);
        Ok(())
    }
    fn follow<D, T>(
        &mut self,
        value: &Value,
        delta: &mut D,
        thinking: &mut T,
    ) -> Result<bool, TransportFailure>
    where
        D: FnMut(String) -> ControlFlow<()>,
        T: FnMut(String) -> ControlFlow<()>,
    {
        match value["type"].as_str() {
            Some("assistant-stream") => {
                let frame = &value["frame"];
                match frame["type"].as_str() {
                    Some("start") => {
                        if !self.current.is_empty() {
                            return Err(invalid());
                        }
                        if !self.content.is_empty() {
                            self.append("\n\n", delta)?;
                        }
                        self.attempt =
                            Some(frame["attemptId"].as_str().ok_or_else(invalid)?.to_owned());
                        self.next_index = 0;
                    }
                    Some("chunk") => {
                        if self.attempt.as_deref() != frame["attemptId"].as_str()
                            || frame["index"].as_u64() != Some(self.next_index)
                        {
                            return Err(invalid());
                        }
                        self.next_index += 1;
                        let chunk = &frame["chunk"];
                        match chunk["type"].as_str() {
                            Some("text-delta") => {
                                let text = chunk["text"].as_str().ok_or_else(invalid)?;
                                self.append(text, delta)?;
                                self.current.push_str(text);
                            }
                            Some("reasoning-delta") => {
                                let text = chunk["text"].as_str().ok_or_else(invalid)?;
                                self.scalars = self.scalars.saturating_add(text.chars().count());
                                if self.scalars > AI_OUTPUT_LIMIT {
                                    return Err(failure("OUTPUT_LIMIT", "DSH 输出超过长度限制"));
                                }
                                if thinking(text.to_owned()).is_break() {
                                    return Err(cancelled());
                                }
                            }
                            _ => {}
                        }
                    }
                    Some("end") => {
                        if frame["outcome"]["kind"] == "abandoned" && !self.current.is_empty() {
                            return Err(failure(
                                "DSH_ATTEMPT_ABANDONED",
                                "DSH 放弃了当前回答，请重试或在 DSH 网页查看会话",
                            ));
                        }
                        self.attempt = None;
                    }
                    _ => return Err(invalid()),
                }
            }
            Some("event") => {
                let event = &value["event"];
                match event["type"].as_str() {
                    Some("assistant/message") => {
                        let content = event["data"]["message"]["content"]
                            .as_array()
                            .ok_or_else(invalid)?;
                        let text: String = content
                            .iter()
                            .filter(|block| block["type"] == "text")
                            .filter_map(|block| block["text"].as_str())
                            .collect();
                        let suffix = text.strip_prefix(&self.current).ok_or_else(invalid)?;
                        self.append(suffix, delta)?;
                        self.current.clear();
                    }
                    Some("assistant/attempt") if !self.current.is_empty() => {
                        return Err(failure(
                            "DSH_ATTEMPT_FAILED",
                            "DSH 当前模型请求未成功提交，请重试或在 DSH 网页查看会话",
                        ));
                    }
                    Some("turn/end") => {
                        return match event["data"]["reason"]["kind"].as_str() {
                            Some("completed") if !self.content.trim().is_empty() => Ok(true),
                            Some("completed") => Err(failure(
                                "EMPTY_RESPONSE",
                                "DSH 未返回文本回答，请在 DSH 网页查看工具结果或附件",
                            )),
                            Some("aborted") => Err(failure("DSH_ABORTED", "DSH 会话已被中断")),
                            Some("max-tokens") => {
                                Err(failure("DSH_OUTPUT_TRUNCATED", "DSH 回答达到输出上限"))
                            }
                            Some("error") => Err(failure(
                                "DSH_TURN_ERROR",
                                "DSH 生成失败，请在 DSH 网页查看会话错误与模型配置",
                            )),
                            _ => Err(failure(
                                "DSH_TURN_INCOMPLETE",
                                "DSH 会话未完成，请在 DSH 网页查看状态",
                            )),
                        };
                    }
                    _ => {}
                }
            }
            _ => return Err(invalid()),
        }
        Ok(false)
    }
}

#[derive(Default, Serialize)]
struct DshTiming {
    request_id: String,
    auth_ready_ms: Option<u64>,
    workspace_ready_ms: Option<u64>,
    session_ready_ms: Option<u64>,
    model_ready_ms: Option<u64>,
    socket_ready_ms: Option<u64>,
    follow_ready_ms: Option<u64>,
    attachment_ready_ms: Option<u64>,
    attachment_count: usize,
    prompt_sent_ms: Option<u64>,
    prompt_ack_ms: Option<u64>,
    first_event_ms: Option<u64>,
    first_content_ms: Option<u64>,
    first_reasoning_ms: Option<u64>,
    turn_end_ms: Option<u64>,
    server_turn_ms: Option<u64>,
    total_ms: u64,
    auth_reused: bool,
    workspace_reused: bool,
    session_reused: bool,
    socket_reused: bool,
    prompt_correlated: bool,
    ack_error: Option<&'static str>,
    outcome: &'static str,
}
fn elapsed(budget: &GenerationBudget, at: Instant) -> u64 {
    at.saturating_duration_since(budget.started_at)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

impl DshService {
    #[cfg(test)]
    pub(crate) async fn generate<D, T, N>(
        &self,
        request_id: &str,
        provider: &ProviderConfig,
        key: &str,
        model: &str,
        thinking_plan: &ThinkingRequestPlan,
        messages: &Value,
        cancellation: &CancellationToken,
        budget: &GenerationBudget,
        delta: D,
        thinking: T,
        notice: N,
    ) -> Result<TransportSuccess, TransportFailure>
    where
        D: FnMut(String) -> ControlFlow<()>,
        T: FnMut(String) -> ControlFlow<()>,
        N: FnMut(String),
    {
        self.generate_with_attachments(
            request_id,
            provider,
            key,
            model,
            thinking_plan,
            messages,
            &[],
            cancellation,
            budget,
            delta,
            thinking,
            notice,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn generate_with_attachments<D, T, N>(
        &self,
        request_id: &str,
        provider: &ProviderConfig,
        key: &str,
        model: &str,
        thinking_plan: &ThinkingRequestPlan,
        messages: &Value,
        attachments: &[crate::quick_ask::Attachment],
        cancellation: &CancellationToken,
        budget: &GenerationBudget,
        delta: D,
        thinking: T,
        notice: N,
    ) -> Result<TransportSuccess, TransportFailure>
    where
        D: FnMut(String) -> ControlFlow<()>,
        T: FnMut(String) -> ControlFlow<()>,
        N: FnMut(String),
    {
        self.generate_in_conversation(
            &DshConversation::default(),
            request_id,
            provider,
            key,
            model,
            thinking_plan,
            messages,
            attachments,
            attachments,
            cancellation,
            budget,
            delta,
            thinking,
            notice,
        )
        .await
    }

    pub(crate) async fn generate_in_conversation<D, T, N>(
        &self,
        conversation: &DshConversation,
        request_id: &str,
        provider: &ProviderConfig,
        key: &str,
        model: &str,
        thinking_plan: &ThinkingRequestPlan,
        messages: &Value,
        attachments: &[crate::quick_ask::Attachment],
        latest_attachments: &[crate::quick_ask::Attachment],
        cancellation: &CancellationToken,
        budget: &GenerationBudget,
        mut delta: D,
        mut thinking: T,
        mut notice: N,
    ) -> Result<TransportSuccess, TransportFailure>
    where
        D: FnMut(String) -> ControlFlow<()>,
        T: FnMut(String) -> ControlFlow<()>,
        N: FnMut(String),
    {
        let mut remote = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(cancelled()),
            _ = tokio::time::sleep_until(budget.total_deadline) => return Err(failure("TOTAL_TIMEOUT", "DSH 生成超时")),
            state = conversation.state.lock() => state,
        };
        let workspace = provider.dsh_workspace.as_deref().unwrap_or("/workspace");
        let compatible = remote.as_ref().is_some_and(|state| {
            state.provider_id == provider.id
                && state
                    .gateway
                    .authentication_matches(&provider.base_url, key)
                && state.workspace == workspace
        });
        let gateway = if compatible {
            remote.as_ref().ok_or_else(invalid)?.gateway.clone()
        } else {
            self.gateway(provider, key)?
        };
        let (upstream_provider, upstream_model) = split_model_id(model)?;
        let initialized = compatible && remote.as_ref().is_some_and(|state| state.initialized);
        let system = system_text(messages)?;
        let prompt = if initialized {
            incremental_prompt(messages, &remote.as_ref().ok_or_else(invalid)?.system)?
        } else {
            prompt_text(messages)?
        };
        let attachments = if initialized {
            latest_attachments
        } else {
            attachments
        };
        crate::quick_ask::validate_attachments(attachments)
            .map_err(|message| failure("DSH_ATTACHMENT_INVALID", message))?;
        let rpc = &gateway.rpc;
        let mut session_id = if compatible {
            remote.as_ref().map(|state| state.session_id.clone())
        } else {
            None
        };
        let mut output = Output::default();
        let mut timing = DshTiming {
            request_id: request_id.to_owned(),
            ..Default::default()
        };
        let operation = async {
            timing.auth_reused = rpc.authenticate().await?;
            timing.auth_ready_ms = Some(elapsed(budget, Instant::now()));
            // Model selection and WebSocket readiness are independent. Preserve
            // the window's remote Agent across turns and model changes.
            let prepare_session = async {
                let id = if let Some(id) = session_id.as_ref() {
                    timing.session_reused = true;
                    id.clone()
                } else {
                    let (workspace_id, reused) = gateway
                        .workspace(provider.dsh_workspace.as_deref().unwrap_or("/workspace"))
                        .await?;
                    timing.workspace_reused = reused;
                    timing.workspace_ready_ms = Some(elapsed(budget, Instant::now()));
                    let created = match rpc
                        .call(
                            "session/create",
                            json!({"request":{"workspaceId":workspace_id}}),
                        )
                        .await
                    {
                        Ok(created) => created,
                        Err(error) => {
                            gateway.invalidate_workspaces().await;
                            return Err(error);
                        }
                    };
                    let id = created["sessionId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(invalid)?
                        .to_owned();
                    session_id = Some(id.clone());
                    *remote = Some(RemoteConversation {
                        gateway: gateway.clone(),
                        provider_id: provider.id.clone(),
                        workspace: workspace.to_owned(),
                        session_id: id.clone(),
                        system: None,
                        initialized: false,
                    });
                    id
                };
                timing.session_ready_ms = Some(elapsed(budget, Instant::now()));
                notice(format!("DSH 会话：{}（可在 {} 查看）", id, rpc.base));
                let mut selection =
                    json!({"sessionId":id, "provider":upstream_provider, "model":upstream_model});
                let effort = match thinking_plan.mode {
                    ThinkingMode::Off
                        if thinking_plan
                            .capability
                            .as_ref()
                            .is_some_and(|cap| cap.supports_off) =>
                    {
                        Some("none")
                    }
                    ThinkingMode::Off => None,
                    ThinkingMode::Minimal => Some("minimal"),
                    ThinkingMode::Low => Some("low"),
                    ThinkingMode::Medium => Some("medium"),
                    ThinkingMode::High => Some("high"),
                    ThinkingMode::Xhigh => Some("xhigh"),
                };
                if let Some(effort) = effort {
                    selection["reasoningEffort"] = json!(effort);
                }
                rpc.call("session/selectModel", json!({"request":selection}))
                    .await?;
                timing.model_ready_ms = Some(elapsed(budget, Instant::now()));
                Ok::<(), TransportFailure>(())
            };
            let ((mux, reused, ready_at), ()) = tokio::try_join!(gateway.mux(), prepare_session)?;
            timing.socket_reused = reused;
            timing.socket_ready_ms = Some(elapsed(budget, ready_at));
            let id = session_id.as_ref().ok_or_else(invalid)?;
            let mut subscription = mux.follow(id)?;
            let opening = async {
                loop {
                    match subscription.receiver.recv().await {
                        Some(MuxItem::Value(value)) if value["type"] == "snapshot" => break,
                        Some(MuxItem::Notice(message)) => notice(message),
                        _ => return Err(failure("DSH_STREAM_ERROR", "DSH 会话订阅失败，请重试")),
                    }
                }
                Ok::<(), TransportFailure>(())
            };
            tokio::time::timeout(RPC_TIMEOUT, opening)
                .await
                .map_err(|_| failure("DSH_STREAM_TIMEOUT", "DSH 会话订阅就绪超时"))??;
            timing.follow_ready_ms = Some(elapsed(budget, Instant::now()));
            let content =
                attachments::prompt_content(rpc, id, prompt, attachments, &mut notice).await?;
            timing.attachment_ready_ms = Some(elapsed(budget, Instant::now()));
            timing.attachment_count = attachments.len();
            let args = json!({"request":{"sessionId":id, "requestId":request_id,
                "mode":"queue", "content":content}});
            let prompt_call = rpc.call("session/prompt", args);
            tokio::pin!(prompt_call);
            timing.prompt_sent_ms = Some(elapsed(budget, Instant::now()));
            let mut acknowledged = false;
            let mut turn_complete = false;
            let mut server_turn_start = None;
            loop {
                // Prompt acceptance and streamed output can arrive in either order.
                // A durable message correlated by requestId proves admission even
                // if the HTTP acknowledgement is still pending or was lost.
                tokio::select! {
                    biased;
                    item = subscription.receiver.recv(), if !turn_complete => {
                        let value = match item {
                            Some(MuxItem::Value(value)) => value,
                            Some(MuxItem::Notice(message)) => { notice(message); continue; }
                            _ => return Err(failure("DSH_DISCONNECTED", "DSH 连接已断开，请重试；当前任务不会自动重发")),
                        };
                        let now = elapsed(budget, Instant::now());
                        timing.first_event_ms.get_or_insert(now);
                        if value["type"] == "event" {
                            let event = &value["event"];
                            if event["type"] == "turn/start" { server_turn_start = event["time"].as_u64(); }
                            if event["type"] == "user/message" && event["data"]["source"]["rpcId"] == request_id {
                                timing.prompt_correlated = true;
                                if let Some(state) = remote.as_mut() {
                                    state.initialized = true;
                                    state.system = system.clone();
                                }
                            }
                            // Late events from a previous turn must not finish this request.
                            if !timing.prompt_correlated { continue; }
                            if event["type"] == "tool/call" {
                                let name = event["data"]["name"].as_str().unwrap_or("工具");
                                notice(format!("DSH 正在调用 {}", name.chars().take(80).collect::<String>()));
                            }
                            if event["type"] == "turn/end" {
                                timing.turn_end_ms = Some(now);
                                timing.server_turn_ms = server_turn_start.and_then(|start| event["time"].as_u64().and_then(|end| end.checked_sub(start)));
                            }
                        }
                        if !timing.prompt_correlated { continue; }
                        if value["type"] == "assistant-stream" && value["frame"]["chunk"]["type"] == "reasoning-delta" {
                            timing.first_reasoning_ms.get_or_insert(now);
                        }
                        let before = output.content.len();
                        turn_complete = output.follow(&value, &mut delta, &mut thinking)?;
                        if output.content.len() > before { timing.first_content_ms.get_or_insert(now); }
                        if turn_complete && (acknowledged || timing.prompt_correlated) { return Ok(()); }
                    }
                    receipt = &mut prompt_call, if !acknowledged => {
                        timing.prompt_ack_ms = Some(elapsed(budget, Instant::now()));
                        match receipt {
                            Ok(value) if value["accepted"] == true => {
                                acknowledged = true;
                                if let Some(state) = remote.as_mut() {
                                    state.initialized = true;
                                    state.system = system.clone();
                                }
                            },
                            Ok(_) => return Err(invalid()),
                            Err(error) if timing.prompt_correlated => { timing.ack_error = Some(error.code); acknowledged = true; }
                            Err(error) => return Err(error),
                        }
                        if turn_complete { return Ok(()); }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(120)), if !turn_complete => {
                        return Err(failure("DSH_STREAM_TIMEOUT", "DSH 长时间没有返回事件，请在 DSH 网页检查待处理的审批或问题"));
                    }
                }
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(cancelled()),
            _ = tokio::time::sleep_until(budget.total_deadline) => Err(failure("TOTAL_TIMEOUT", "DSH 生成超时，请在 DSH 网页检查状态")),
            result = operation => result,
        };
        if result.is_err() {
            if let Some(id) = session_id.as_ref() {
                let _ = tokio::time::timeout(
                    Duration::from_secs(3),
                    rpc.call("session/cancel", json!({"request":{"sessionId":id}})),
                )
                .await;
            }
        }
        timing.total_ms = elapsed(budget, Instant::now());
        timing.outcome = result
            .as_ref()
            .err()
            .map(|error| error.code)
            .unwrap_or("completed");
        if let Ok(record) = serde_json::to_string(&timing) {
            eprintln!("[dsh-network] {record}");
        }
        if let Err(mut error) = result {
            if !attachments.is_empty()
                && timing.prompt_sent_ms.is_none()
                && error.code == "TOTAL_TIMEOUT"
            {
                error.retryable = true;
            }
            error.content_seen = !output.content.is_empty();
            return Err(error);
        }
        Ok(TransportSuccess {
            content: output.content,
            transport: ResponseTransport::Dsh,
            termination: ResponseTermination::TurnEnd,
        })
    }
}

#[cfg(test)]
#[path = "dsh_transport_tests.rs"]
mod tests;
