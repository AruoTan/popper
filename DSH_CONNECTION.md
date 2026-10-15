# DSH Docker 接入

Popper 的「AI 服务商与模型」现在支持两种接入方式：OpenAI 兼容接口和 DSH Docker。旧配置继续使用 OpenAI 兼容接口。

## 安装包与调用协议

所给 `dsh-docker-backend.7z` 是一个 Node.js 网关与 Docker Compose 部署项目，固定安装 `@deepseek-ai/dsh@0.2.0-rc.2`。本地免填密钥时，Popper 使用网关的自动登录获取 Cookie；填写 `DSH_API_KEY` 时，网关将 Bearer 鉴权转换为私有 DSH 的 Cookie 鉴权。网页与外部应用共享模型配置、工作区和会话。默认宿主地址是 `http://localhost:3080`。

本次按固定版本的官方协议接入，不能把该网关当作 OpenAI `/v1/chat/completions` 服务。

| 用途 | 原生接口 |
| --- | --- |
| 获取模型与连接检查 | HTTP POST `/api/session/modelCatalog` |
| 注册容器工作区 | HTTP POST `/api/workspace/create` |
| 创建会话、选模型、提交消息、停止 | HTTP POST `/api/session/create`、`selectModel`、`prompt`、`cancel` |
| 回答与推理流 | WebSocket `/api/remote.mux`，先打开 `$events`，等 `ready` 后打开 `session/follow`，等 `snapshot` 后提交消息 |
| 将审批与问题交给 DSH 网页 | HTTP POST `/api/$events/result`，`outcome.kind=next` |

HTTP 请求使用官方 `client-request` 信封，检查 HTTP 状态、`rpcId`、`result.ok`。流按照 rc.2 的 `event` 和 `assistant-stream` 帧处理：实时文本与最终 `assistant/message` 对齐去重，以成功的 `turn/end` 确认完成。附件示例中的旧式 `entry` 判断不适用于这个版本；这里以固定版本的官方源码为准。

## 配置步骤

1. 按安装包的说明启动容器，在 DSH 网页的「设置 → 模型」中保存模型供应商凭据，并确认模型可以回答。
2. 在 Popper 设置中添加服务商，展开配置，把「接入方式」选为 **DSH Docker**。
3. API 地址填 `http://localhost:3080`（远程部署填对应网关地址），不附加 `/api` 或 `/v1`。
4. 使用 `localhost`、`127.0.0.1` 或 `[::1]` 时，**API Key 可留空**。Popper 自动访问同一网关的根页面，获取登录 Cookie，并用于 HTTP RPC 与 WebSocket。远程地址需填安装包 `.env` 的 **DSH_API_KEY**。DeepSeek 等供应商的模型密钥保存在 DSH 中。
5. DSH 工作区填容器内绝对路径，默认 `/workspace`。这不是 Popper 所在电脑的文件路径。
6. 点击「获取模型」，选择模型并保存，再在工具栏动作中选择这个服务商的模型。

模型 ID 保存为 `DSH提供商ID/模型ID`，例如 `deepseek/model-name`；模型本身含 `/` 时保留完整后缀。同名模型可以来自不同 DSH 提供商。手动添加也需使用该格式。思考强度来自 DSH 的目录元数据；未声明的档位不会发送，关闭思考仅在 DSH 明确声明 `none` 时传入，否则使用 DSH 默认值。

本地免填密钥依赖安装包默认的 `DSH_LOCAL_GUI_AUTO_LOGIN=true`，宿主发布地址与 `DSH_PUBLIC_URL` 都需为本地地址。关闭自动登录时，可以填写 `DSH_API_KEY`。安装包服务端的 `DSH_API_KEY` 配置仍需保留；免填指 Popper 本地连接不需要手工提供该密钥。登录 Cookie 仅用于当前网关，不随重定向发送到其他地址，也不会写入普通设置。

更换接入方式会重置该服务商的地址、工作区、模型列表以及动作的旧模型引用；保存前可重新填写。已有 API Key 仍可编辑，请替换为目标服务的凭据。

## 会话与交互

- 同一个 Popper 结果窗口复用一个 DSH 会话，包括生成、追问、手动重试及在同一 DSH 服务商内换模型。首次提交任务与上下文，此后只提交最新用户消息和本轮附件，历史回答、工具结果及已上传文件由 DSH 保留，不再由 Popper 重发。任务指令发生变化（例如翻译后继续提问）时，仅补充新的指令。
- 手动重试是在同一 DSH 会话中再次提交当前问题，远端历史不会回滚。不同结果窗口使用独立会话；关闭窗口或退出 Popper 后丢弃本地绑定，DSH 中的远端会话仍可查看。更换服务商、网关地址、密钥或工作区时建立新会话并提交当前上下文；切换到 OpenAI 兼容接入后，返回 DSH 也会重新建立绑定。
- 文本与推理在 Popper 中流式显示，工具调用显示状态提示。需要审批或回答问题时，提示包含网关地址和会话 ID；打开同一实例的 DSH 网页并进入该会话处理。Popper 委派交互，不自动批准工具操作。
- 停止、超时、协议错误或连接断开会尝试调用远端 `session/cancel`。取消最多等待 3 秒；网络不可达时请在 DSH 网页确认执行状态。断线时不自动重发旧消息，下次请求会建立新连接。
- Popper 展示文本和推理，不呈现 DSH 的完整附件、工具结果卡片或交互表单。没有文本的结果会提示在 DSH 网页查看。被放弃的已有文本片段、失败或被截断的回合会报告错误，不标为完成。
- 「获取模型」核验 HTTP 鉴权和模型目录；「测试连接」额外建立 WebSocket、等待 `$events/ready` 后关闭，不创建 Agent 或产生模型请求。模型凭据是否有效需要实际生成验证。反向代理需要同时支持 HTTP RPC 和 WebSocket Upgrade。

## 会话流连接失败

如果「获取模型」成功而生成失败，HTTP 可用并不代表 `/api/remote.mux` 的 WebSocket 握手可用。先点击「测试连接」，查看具体错误：HTTP 401 为认证拒绝，403 为访问边界拒绝，404 为路径未匹配，502/503/504 为网关或上游不可用。重定向、网络及 HTTPS 连接错误、握手响应缺失头和超时会提示；超时信息区分等待 Upgrade 响应与接管升级连接，错误信息不包含密钥、Cookie 或响应正文。

WebSocket 通过与 RPC 相同的 HTTP 客户端发送 HTTP/1.1 Upgrade，共用 DNS 解析、IPv4/IPv6 快速回退、代理、TLS 信任及连接池。此前 WebSocket 单独直连并逐个尝试解析地址，HTTP 能到达的网关与 WebSocket 的连接路径可能不同，导致仅会话流超时。升级时仍校验状态 101、Upgrade、Connection 和 Sec-WebSocket-Accept，拒绝未请求的子协议及扩展；握手等待上限仍为 15 秒。

本地地址留空 API Key 即可。修改 `localhost`、`127.0.0.1` 或端口后，Popper 会重新获取与新地址绑定的 Cookie，不沿用旧地址凭据。修改上述 Rust 后端后需要重新编译并运行桌面程序，单独刷新前端页面不会更新连接实现。

本次另用安装包中的原始 Node 网关与模拟 DSH 上游完成 Cookie 和 Bearer 两种 HTTP/WS 流式联调；这验证网关转发与 Popper 传输兼容性，不替代用户机器上的 Docker、网络和真实模型验证。

连接路径专项测试在本地创建一个接受 TCP 但不回复握手的端口，让 HTTP 经代理访问另一个正常网关。旧实现获取模型成功、WebSocket 等待 15 秒超时；新实现走相同代理并完成流式生成。另验证 HTTP 与 WebSocket 共用客户端 DNS 设置，以及升级后的流在 HTTP 请求超时期限之后仍能使用。这些是模拟连接验证，不代表真实模型推理用时。

## 响应延迟优化与计时

同一服务商复用 HTTP 客户端、登录 Cookie、工作区 ID 和 WebSocket；同一结果窗口还复用 Agent 会话。首次创建会话及每轮选择模型与准备 WebSocket 并行进行。网关资源缓存过期或流连接空闲释放，不会改变当前结果窗口绑定的会话 ID。

提交消息的 HTTP 确认与回答流并行处理。已收到匹配本次 `requestId` 的持久用户消息和成功 `turn/end` 时，确认响应尚未到达也可完成，不因慢确认延迟显示。审批委派在独立任务中执行，不阻塞文本流。多个结果窗口通过不同订阅 ID 接收各自会话的输出，停止其中一个只取消对应订阅和 Agent。

地址或密钥变化会使用独立缓存；工作区按路径缓存。Cookie 的 HTTP 401 拒绝会触发一次重新登录，这是请求尚未进入 RPC 时的认证重试；网络超时、断线或业务失败不会自动重发任务。连接空闲 5 分钟后释放，缓存规模和输出队列都有上限。

后端日志新增 `[dsh-network]` JSON 行，通过 `request_id` 与普通 `[network]` 日志对应。所有客户端 `*_ms` 为从后端动作开始到该阶段的累计毫秒数，不包含动作进入后端之前的窗口启动时间。

| 字段 | 含义 |
| --- | --- |
| `auth_ready_ms` / `workspace_ready_ms` | 凭据和工作区准备完成 |
| `session_ready_ms` / `model_ready_ms` | 会话创建或复用与模型选择完成 |
| `socket_ready_ms` / `follow_ready_ms` | WebSocket 与会话订阅就绪 |
| `prompt_sent_ms` / `prompt_ack_ms` | 开始提交消息 / 收到 HTTP 确认；提前完成时确认时间可能为空 |
| `first_event_ms` / `first_content_ms` / `first_reasoning_ms` | 首个会话事件、首个正文、首个推理片段 |
| `turn_end_ms` / `total_ms` | 收到回合结束 / 客户端请求结束 |
| `server_turn_ms` | DSH `turn/start` 与 `turn/end` 事件时间戳的差值，字段缺失时为空 |
| `auth_reused` / `workspace_reused` / `session_reused` / `socket_reused` | 本次是否复用了相应资源；复用会话时无需查询工作区 |

准备时间主要看 `prompt_sent_ms`，提交到正文的等待看 `first_content_ms - prompt_sent_ms`。`server_turn_ms` 可帮助区分 DSH 回合执行时间与 Popper 请求总时间，但这些日志不等同于屏幕绘制时间。日志不记录提示词、回答、密钥或 Cookie。

## 保存临时会话

安装并启用 `dsh-temporary-sessions` 插件的网关会将应用新会话放入独立临时运行时，与普通 GUI 历史、附件和会话事件隔离；profile 和模型凭据共享。临时会话按最后真实对话活动计算，超过 24 小时且没有正在进行的工作时，在北京时间每日零点检查中清理，启动时补查。Popper 的 RPC、上传和 WebSocket 均发送 `X-DSH-Client: app`，本地 Cookie 与 Bearer 两种鉴权均支持该路由。

使用 DSH 的结果窗口顶部显示“保存会话”。发送消息创建远端会话后，可在生成结束或取消后点击；保存中禁止重试、换模和追问，成功后显示“已保存”。普通 AI API 会话及词典模式不显示该按钮。窗口重新连接时从后端恢复保存状态。

按钮调用原生 RPC `POST /api/temporary/save`，参数为 `payload.args.request.sessionId`，由后端当前窗口的会话绑定确定，前端不提交任意远端 ID。插件先通过原生归档准入冻结会话，保留原 ID、事件顺序、模型配置及子 Agent 历史，复制并验证引用的附件，再导入普通运行时并发布 GUI 历史事件。活动回合、子任务、后台工作或提醒会返回 `temporary/session-active`，Popper 显示提示并允许之后手动重试；未提供接口的旧网关会提示安装插件。

保存成功后，Popper 为该会话创建独立的新事件连接（重新订阅 `$events` 和 `session/follow`），下一轮通过 `session/create` 仅传原 `sessionId` 恢复，避免使用临时运行时的工作区 ID。后续消息继续进入同一普通会话，不重放历史，不重新上传旧附件，也不重新触发模型生成。其他窗口的临时流不因客户端主动重建而关闭。若保存回执丢失，可再次点击，插件以原 ID 幂等返回；客户端也会在后续请求前重连并按 ID 恢复，避免使用已转正的旧连接。

桌面验收：在 DSH 临时 GUI `/temporary/` 中确认 Popper 会话；点击“保存会话”后，在普通 GUI 确认相同 ID 的完整历史及附件；在 Popper 继续提问，确认消息追加到该普通会话。保存前保留另一个窗口，确认其仍能继续使用。真实插件容器及桌面端联调仍需在安装环境执行。

## 验证与参考

新增模拟网关测试覆盖原生 RPC/WS 路由、模型提供商区分、实时回答/推理、最终消息去重、审批委派、停止时取消远端 Agent 和业务错误脱敏，以及本地 Cookie 登录、远程密钥要求和登录失败不跟随重定向。前端测试覆盖接入方式切换、设置保存、旧配置兼容和本地无密钥时的模型选择与换模。

优化专项回归另覆盖多轮资源复用、慢确认、并发回答隔离、单独取消、Cookie 失效、地址与密钥变化、工作区变化和空闲释放。模拟网关中，两轮生成共用一次登录、一次工作区注册和一个 WebSocket；该结果验证资源复用，不代表真实模型推理提速的秒数。

会话复用回归使用模拟 HTTP/WS 网关，验证 Cookie 与 Bearer 两种鉴权下的多轮提问、换模、重试均只创建一次会话；历史内容不重发，只有本轮新附件上传。另覆盖目标配置变化后的上下文重建、资源缓存过期、停止后重试、旧回合事件隔离、等待会话锁时的取消和超时，以及结果窗口关闭后清理绑定。

桌面验收：在同一结果窗口连续提问三轮并换模型，检查 DSH 网页只新增一个会话，且后两轮消息不包含整段历史；上传文件后追问不再上传该文件，新附件仅在对应回合出现。开启第二个结果窗口应建立另一个会话；停止其中一个窗口的生成不影响另一个。后端 `[dsh-network]` 的后续回合应显示 `session_reused: true`。这些真实 Docker/模型行为仍需桌面实测。

本次验证通过前端类型检查、生产构建及相关回归测试，并通过平台独立 Rust 模块测试。另用临时桌面事件桩检查动作层编译和状态机测试；这不替代真实 Tauri 平台验证。当前环境没有 Docker 与可用模型凭据，尚未进行真实容器模型联调；完整桌面编译也因缺少平台构建工具而未验证。

固定版本官方资料：

- [rc.2 Connection 协议](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/client/connection/README.md)
- [rc.2 Session API 与类型](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/api/session-controller/src/types.ts)
- [rc.2 Session API 实现](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/api/session-controller/src/index.ts)
- [rc.2 Gateway 流协议](https://github.com/deepseek-ai/deepseek-harness/blob/dsh-v0.2.0-rc.2/packages/api/gateway/src/stream-protocol.ts)

DSH 处于开发预览阶段，升级版本后需要重新核对协议。
