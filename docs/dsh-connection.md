# DSH Docker 接入

[使用指南](./user-guide.md#配置模型与动作) · [开发手册](./development.md)

## 配置

Popper 使用 DSH 原生协议，对接 `@deepseek-ai/dsh@0.2.0-rc.2` 网关，不使用 OpenAI Chat Completions 路径。

1. 启动 DSH 网关，在其网页设置中配置模型凭据，并确认模型可以回答。
2. 在 Popper“通用 → AI 服务商与模型”中添加服务商，将接入方式选为“DSH Docker”。
3. 填写网关根地址，默认 `http://localhost:3080`，不附加 `/api` 或 `/v1`。
4. 本地 `localhost`、`127.0.0.1`、`[::1]` 可留空 API Key，使用网关自动登录；远程地址填写网关的 `DSH_API_KEY`。
5. 工作区填写容器内绝对路径，默认 `/workspace`。
6. 获取模型并保存，在“动作”中选择默认模型或为具体动作指定模型。

本地免密连接要求网关启用 `DSH_LOCAL_GUI_AUTO_LOGIN=true`，发布地址与 `DSH_PUBLIC_URL` 均为本地地址；关闭自动登录时填写网关密钥。模型供应商的密钥保存在 DSH，不填入 Popper 的网关密钥栏。

模型 ID 为 `提供商ID/模型ID`。思考档位来自模型目录，只发送 DSH 声明支持的值。更换接入方式会重置地址、工作区、模型列表与动作模型引用，需要重新配置。

## 会话

同一结果窗口复用一个 DSH 会话。首次提交上下文，后续只发送最新问题和本轮新附件；换模型与重试继续使用该会话，重试不回滚远端历史。不同窗口互相独立。更换服务商、地址、密钥或工作区后重新建立会话。

Popper 流式显示正文、推理和工具状态。需要审批或回答交互问题时，前往同一网关的 DSH 网页，打开提示中的会话处理。停止、超时或断线会尝试取消远端任务；无法连接时需在 DSH 网页确认执行状态，不自动重发旧消息。

关闭 Popper 结果窗口只丢弃本地绑定，远端会话由 DSH 管理。普通文件上传到对应会话，图片使用原生图片消息；解析能力取决于模型和网关工具。

## 保存临时会话

网关启用 `dsh-temporary-sessions` 后，Popper 新会话进入临时运行时。超过 24 小时且无活动工作的会话由插件按其清理计划删除；需要保留时，在结果窗口生成结束或取消后点击“保存会话”。

保存成功后，会话以原 ID 转入普通历史，后续提问继续追加，不重新上传旧附件。活动回合、子任务或后台工作会阻止保存，可结束后重试。网关未提供保存接口时，Popper 提示安装插件。

## 连接排查

“获取模型”只验证 HTTP 鉴权和模型目录；“测试连接”还验证 WebSocket，不调用模型。实际生成才能确认模型凭据可用。反向代理需同时转发 HTTP RPC 与 `/api/remote.mux` 的 WebSocket Upgrade。

| 错误 | 检查 |
| --- | --- |
| 401 | 网关密钥、本地自动登录与 Cookie |
| 403 | 网关访问边界 |
| 404 | 地址是否为网关根地址，代理是否保留原路径 |
| 502／503／504 | 反向代理与上游服务状态 |
| 模型目录正常，生成失败 | WebSocket Upgrade、TLS、代理与会话流错误 |
| 握手超时 | 网关和代理是否支持 HTTP/1.1 Upgrade；等待上限 15 秒 |

HTTP 与 WebSocket 共用客户端的代理、DNS 和 TLS 设置。本地地址或端口变更后重新登录，Cookie 不跨网关或重定向发送。错误日志不包含密钥、Cookie 和响应正文。

## 开发接口

| 用途 | 接口 |
| --- | --- |
| 模型目录 | `POST /api/session/modelCatalog` |
| 工作区 | `POST /api/workspace/create` |
| 会话与生成 | `POST /api/session/create`、`selectModel`、`prompt`、`cancel` |
| 事件流 | WebSocket `/api/remote.mux`，`$events` 就绪后订阅 `session/follow` |
| 交互委派 | `POST /api/$events/result`，`outcome.kind=next` |
| 保存临时会话 | `POST /api/temporary/save` |

RPC 使用 `client-request` 信封并校验 `rpcId` 与 `result.ok`。等待 `session/follow` 的快照后提交问题；按请求隔离流事件，与最终消息对齐去重，只以成功的 `turn/end` 确认完成。保存请求的会话 ID 从后端窗口绑定取得，不接受前端任意 ID。RPC、上传和 WebSocket 携带 `X-DSH-Client: app`。

同一服务商复用 HTTP 客户端、凭据、工作区与 WebSocket；空闲连接在 5 分钟后释放。会话复用独立于连接缓存。停止一个窗口只取消其任务与订阅，不能影响其他窗口。

`[dsh-network]` 日志用 `request_id` 关联请求，客户端 `*_ms` 从后端动作开始累计。`prompt_sent_ms` 表示准备完成，`first_content_ms - prompt_sent_ms` 表示提交后等待正文的时间，`server_turn_ms` 表示服务端回合时间。复用状态见 `auth_reused`、`workspace_reused`、`session_reused`、`socket_reused`。

协议依据：[Connection](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/client/connection/README.md)、[Session API](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/api/session-controller/src/types.ts)、[流协议](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/api/gateway/src/stream-protocol.ts)。升级 DSH 时重新核对协议，并验证多轮复用、附件、换模、取消、并发窗口与临时会话保存。
