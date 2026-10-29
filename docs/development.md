# Popper 开发手册

[项目首页](../README.md) · [使用指南](./user-guide.md) · [Windows 构建与发布](./build-and-release.md)

## 环境

Popper 使用 Tauri 2、Rust、React 和 TypeScript。前端与业务逻辑共享，系统选区、窗口、剪贴板与屏幕捕捉通过平台实现接入。

| 环境 | 要求 |
| --- | --- |
| 通用 | Git 完整历史与标签、Node.js 22.12+、`package.json` 指定的 pnpm |
| Windows | Windows 10/11 x64、Rust MSVC、Visual Studio C++ Build Tools、Windows SDK、NASM、WebView2 |
| macOS | macOS 12+、Apple Silicon、Rust、Xcode Command Line Tools |
| 开发容器 | Docker 与 Dev Containers；用于前端验证和 Rust 编辑 |

CI 和容器固定 Node 22.22.0、Rust 1.97.1。使用锁文件安装依赖：

```sh
pnpm install --frozen-lockfile
```

Windows 在 Visual Studio 开发终端运行，先安装 NASM 并确认 `nasm -v` 可用。CI 自动下载并校验 NASM。

```powershell
rustup target add x86_64-pc-windows-msvc
pnpm dev:windows
```

macOS 安装 Command Line Tools 并启动：

```sh
xcode-select --install
rustup target add aarch64-apple-darwin
pnpm dev
```

开发应用从托盘或菜单栏打开设置。纯前端预览使用 `pnpm dev:web`，入口为 `http://localhost:1420/settings/index.html`；浏览器不提供 Tauri IPC，不能验证系统操作。容器配置见[开发容器](../.devcontainer/README.md)。

## 代码结构

| 路径 | 职责 |
| --- | --- |
| `src/shared/` | 数据契约、校验、默认设置、提示词和公共工具 |
| `src/renderer/toolbar/` | 划词工具栏 |
| `src/renderer/result/` | 结果窗口、词典、多轮对话和流式显示 |
| `src/renderer/quick-ask/` | 快捷提问、草稿和附件读取 |
| `src/renderer/settings/` | 设置界面与快捷键录制 |
| `src/renderer/capture/` | 截图、标注、长图预览和贴图界面 |
| `src-tauri/src/` | 生命周期、窗口协调、动作、设置、密钥、网络与平台入口 |
| `src-tauri/src/capture/` | 截图会话、图像、识别、拼接、贴图和 Windows 适配 |
| `apps/windows/` | Windows 划词后端、启动提示、图标、便携打包与产物校验 |
| `apps/macos/` | 原生选区桥接、图标与产物校验 |
| `scripts/` | 版本推导、Tauri 启动与源码导出 |
| `docs/` | 使用、开发与构建发布文档 |
| `licenses/` | 第三方声明与许可证原文 |
| `.github/` | Windows 验证、构建与发布工作流 |

全局选区事件经过平台捕获与 Rust 校验后显示工具栏。用户执行动作时，后端建立结果会话，将词典结果或模型流定向发送给对应窗口。共享数据结构位于 `src/shared/schemas.ts` 和 `src-tauri/src/models.rs`，IPC 变更需要同步两端的类型、校验与调用窗口权限。

## 实现约束

### 窗口与会话

设置窗口按需创建，关闭后销毁。截图结束时销毁覆盖窗口；贴图关闭时销毁图片窗口和所属工具条，最近关闭项只保留 Rust 内存数据，恢复时创建新窗口。显隐操作保留现有贴图窗口。

窗口创建、原生样式与焦点操作在 UI 线程执行。截图窗口通过 Tauri `destroy()` 向事件循环提交销毁，不在 `with_webview` 回调内同步关闭原生控制器，不让 UI 线程等待自身任务。取消与迟到任务按会话标识校验，旧任务不能显示新窗口或恢复旧焦点。

结果事件存储同步发布并恢复乱序事件，展示层负责文字动画。默认调度回调保持稳定；终态立即显示完整结果，重试、结束与卸载清理帧任务。

### 设置与密钥

设置仅支持当前版本 18，拒绝旧版本、缺失字段和未知字段，不覆盖无效配置或密钥。普通设置事务在写锁内规范化、校验并比较，相同值跳过写盘；服务商删除与密钥清理保留完整事务。

API Key 与欧路授权使用本地 AES-256-GCM 加密。公共设置只暴露密钥是否已配置，明文读取只允许设置窗口。日志不记录密钥；DEV Debug 的文本诊断另见 [Windows 平台说明](../apps/windows/README.md#诊断)。

### 附件与内容

快捷提问和结果窗口共用附件批量读取：先检查元数据、单文件 8 MiB 和总量 20 MiB，再读取内容并验证文本合计 60,000 个 Unicode 标量。计数使用公共工具，失败保留原附件与草稿。截图附加到快捷提问需要接收确认，不能提前结束会话或自动发送。

Markdown 不启用原始 HTML，禁用远程图片和危险链接协议。网络请求与外链按各自边界校验；窗口命令同时核验调用者和所属会话。

Windows 划词通过辅助进程隔离可能阻塞的 UIA／OLE 调用，受控复制仅在完整备份后执行，并在剪贴板未被用户修改时恢复。屏幕捕捉的坐标、识别与输出约束见 [屏幕捕捉开发](./screen-capture.md)。

## 验证

通用前端检查：

```sh
pnpm typecheck
pnpm test
pnpm build:web
```

`tsconfig.json` 统一编译检查规则，`tsconfig.node.json` 和 `tsconfig.web.json` 分别指定环境与文件范围。增量检查缓存写入 `node_modules/.cache/typescript/`。`pnpm typecheck` 检查前端与构建配置，包括未使用变量和参数。`pnpm test` 先运行 Node 脚本测试，再运行 Vitest。修改功能时更新行为测试，删除功能时同步清理专用测试、监听和调用路径。

| 平台 | 完整验证 | 原生测试 | 原生编译检查 |
| --- | --- | --- | --- |
| Windows | `pnpm verify:windows` | `pnpm test:rust:windows` | `pnpm check:rust:windows` |
| macOS | `pnpm verify` | `pnpm test:rust` | `pnpm check:rust` |

原生检查需对应平台工具链。Windows CI 执行 Rust 全目标测试后直接打包，不再重复同目标编译检查。桌面验收需要覆盖划词、剪贴板恢复、快捷提问、窗口焦点、多 DPI 与多屏；截图专项清单见 [屏幕捕捉开发](./screen-capture.md#桌面验收)。不能将浏览器测试或跨平台类型检查记录为桌面验收通过。

## 构建

| 产物 | 命令 | 默认目录 |
| --- | --- | --- |
| Windows NSIS | `pnpm package:windows` | `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/` |
| Windows 便携 ZIP | `pnpm package:portable:windows`，先构建 Windows 程序 | 同级 `bundle/portable/` |
| macOS App 与 DMG | `pnpm package` | `src-tauri/target/aarch64-apple-darwin/release/bundle/` |

Windows 完整发布附件通过 `node apps/windows/scripts/prepare-release-assets.mjs` 汇总到 `bundle/distribution/`，包括安装包、便携 ZIP、Scoop 清单和校验文件。设置 `CARGO_TARGET_DIR` 时，产物目录随之改变。当前打包命令生成未签名产物，macOS 未公证。

## 应用版本

`scripts/version.mjs` 从 Git 推导产品版本。标签格式为 `vX.Y.Z`，各段无前导零且不超过 65535；不接受预发布后缀。普通开发使用当前提交最近可达的版本标签，生成元数据同时记录实际提交和距离。查看版本：

```sh
pnpm version:show
node scripts/version.mjs --json
```

Tauri 命令通过 `scripts/tauri.mjs` 生成 `.popper-version.json`；直接运行 Cargo 时由 `src-tauri/build.rs` 调用相同解析器并注入 `POPPER_VERSION`。生成文件被 Git 忽略，内容未变不重写，仓库内始终以 Git 为准。

| 受控内容 | 使用方式 |
| --- | --- |
| 运行时、托盘、启动日志、请求 User-Agent | Tauri 版本或 `POPPER_VERSION` |
| Windows PE／NSIS、macOS Info.plist／DMG | Tauri 读取生成元数据 |
| 便携 ZIP、Scoop、发布附件与校验 | 打包脚本读取统一解析器 |
| 源码导出 | 版本化目录与构建快照 |

`package.json` 和 `Cargo.toml` 不保存产品版本。Cargo 锁文件中的内部 crate 缺省版本 `0.0.0` 仅标识依赖图。依赖版本、配置版本与内部协议版本分别管理。

正式发布要求干净工作区，当前提交恰好有一个有效版本标签。浅克隆执行 `git fetch --unshallow --tags`，完整克隆缺少标签时执行 `git fetch --tags`。发布命令与工作流见 [Windows 构建与发布](./build-and-release.md#发布)。

## 源码导出

```sh
pnpm export:dev-source
```

输出为 `portable-dev-sources/Popper-<version>-dev-source/`，包含源码、锁文件、构建配置、文档、许可证和 `.popper-version.json`。不包含 `.git`、依赖、编译产物、日志、发布包或用户配置。完整清单由 `scripts/export-dev-source.mjs` 管理。

导出包不含 Git 历史，通过快照保留版本来源，可安装依赖后使用相同开发与构建命令，不能直接正式发布。GitHub 自动生成的源码 ZIP 不含此快照，应使用带标签的克隆或本项目导出包。仓库根目录由 `git rev-parse --show-prefix` 判断，避免 Windows 路径别名误判；导出包放入其他仓库的子目录也不会借用外层标签。

## 文档维护

README 负责项目介绍与导航，[使用指南](./user-guide.md) 负责操作，[本手册](./development.md) 负责开发约束与流程，专题文档只保留各自细节。更新行为时同步文档与导出清单；历史变更查 Git 提交和 [Releases](https://github.com/AruoTan/popper/releases)。

根目录 `.gitattributes` 统一容器与 CI 文件的 LF 换行。临时诊断输出写入被忽略的 `.diagnostics/`，可复用工具放在平台脚本目录。图标保留 Windows／macOS 构建资源及 `src-tauri/icons/icon.png`。

权限配置维护在 `src-tauri/capabilities/`；`src-tauri/gen/` 由 Tauri 原生构建生成，不提交、不导出。全新工作区的本地权限 schema 在首次原生构建后可用。

新增源码与文档只描述 Popper 自身功能，不纳入外部参考材料；文件名和文本检查在项目外执行。主许可证保留在根目录 `LICENSE`，第三方声明与许可保存在 `licenses/`；打包时将其映射到分发目录，保留原文件名。依赖归属以锁文件和第三方声明为准。
