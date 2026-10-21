# Windows 云端打包

本地在 Dev Container 开发，GitHub 托管的 `windows-2022` runner 完成 Windows 检查和打包。本机不需要安装 MSVC、Windows SDK、Rust 或 Node。

## 使用

1. 将源码和 `.github` 配置提交、推送到 GitHub 仓库。工作流首次必须存在于默认分支，Actions 页面才会显示手动运行按钮。
2. 打开 **Actions → Build Windows installer → Run workflow**，选择待构建分支并运行。
3. 成功后，在该次运行的 **Artifacts** 下载 `Popper-windows-x64-运行编号`，解压得到安装 `.exe`、便携 `.zip`、Scoop 清单 `popper.json` 和 `SHA256SUMS.txt`。
4. 可用 PowerShell 的 `Get-FileHash .\Popper_0.5.12_x64-setup.exe -Algorithm SHA256` 与校验文件比对（文件名随版本变化）。

此构建工作流可单独手动触发，也可由下面的 Release 工作流调用；不因 push/PR 自动运行。单独构建不会创建 Release、不推送提交、不需要发布令牌或签名密钥。Actions 产物保留 14 天；请及时下载。私有仓库需留意账户的 Actions 分钟数、存储额度及组织策略。

## 手动创建 GitHub Release

1. 将这两个工作流推送到默认分支，然后打开 **Actions → Release Windows installer → Run workflow**。
2. 选择待发布分支。版本号自动从该次提交的 `package.json` 读取，例如 `0.5.12` 对应标签 `v0.5.12`，无需预先创建标签；发布前自行更新项目各处版本号并提交。
3. `draft` 默认勾选，生成 Release 草稿；取消勾选则在附件上传完成后直接发布。`prerelease` 用于标记预发布版本，默认不勾选。
4. 工作流复用完整的 Windows 测试、打包和启动检查，将本次运行的安装 `.exe`、便携 `.zip`、`popper.json` 与 `SHA256SUMS.txt` 上传至 Release，并自动生成更新说明。成功后可从运行摘要打开 Release；草稿在仓库 **Releases** 页面检查并发布。

构建阶段只有 `contents: read`，仅 Release 任务使用 `contents: write` 和 GitHub 自动提供的 `GITHUB_TOKEN`，无需额外配置 PAT。仓库或组织策略需要允许该任务写入仓库内容。

版本检查要求 `package.json`、Tauri 基础/Windows 配置、`Cargo.toml` 和 `Cargo.lock` 一致。标签始终对应本次选择的源码提交；已有标签指向其他提交会失败。已有同名 Release（含草稿）不会被覆盖：需要修改现有 Release 时手动处理，发布新版本时更新版本号。上传或发布中途失败可能留下草稿，可在 Releases 页面检查后处理。

Release 工作流在仓库内串行执行。所有附件由本次运行构建并校验，Release 附件不受 Actions 的 14 天保留期影响。目前包含 Windows x64 安装包及便携版。便携版和 Scoop 使用方式见 [使用说明](../WINDOWS_PORTABLE.md)。

## 流程与维护

- Node 22.22.0、Rust 1.97.1 MSVC，与开发容器版本保持一致；pnpm 精确版本从 `package.json` 读取。
- 通过 `pnpm install --frozen-lockfile` 安装依赖，自动发现 runner 的 Visual Studio/Windows SDK；安装并优先使用 NASM，避免 Strawberry Perl 同名工具干扰。
- 依次运行 `pnpm typecheck`、`pnpm test`、`pnpm build:web` 和 `pnpm test:rust:windows`，成功后复用本次生成的前端产物运行 `pnpm build:windows --config src-tauri/tauri.ci.conf.json '--' --locked`，最后运行 `pnpm verify:artifacts:windows`。沿用已有 PE 架构、版本及 SHA-256 校验，不跳过测试。
- CI 中的 Rust 全目标测试包含编译、链接和测试执行，因此不再紧接着重复运行同目标的 `cargo check --all-targets`；本地 `pnpm verify:windows` 仍保留原有完整命令。全部前端和 Rust 测试仍执行。
- 任一步骤失败都会停止；仅成功产物上传。任务最多运行 60 分钟，同一分支的并发构建不会相互中断。
- Actions 自身使用 Node.js 24 运行时（包括 Rust 缓存）；这与构建项目的 Node 22.22.0 独立。`setup-node` 关闭自动 npm 缓存，pnpm store 由显式缓存步骤管理。
- 第三方 Actions 固定到提交 SHA，构建工作流权限只有 `contents: read`。升级 Actions 时同步核对官方版本与 SHA。
- 缓存 pnpm store，以及 Cargo 下载缓存和 `src-tauri/target` 内的依赖编译产物，覆盖验证与 release 打包。Rust 缓存由工具链、编译环境和 Cargo 清单/锁文件区分；锁文件变化时可回用旧缓存，由 Cargo 判断需要重编译的依赖。应用自身及安装包由当前源码重新生成。
- 仅在 CI 设置 `CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_PROFILE_TEST_DEBUG=0`，减少 Windows 测试构建的调试符号/PDB 生成及缓存体积；保留 debug assertions、溢出检查及 release 构建配置。配置变化后首次运行需要重建对应依赖缓存。
- 同时缓存 `%LOCALAPPDATA%/tauri` 中的 NSIS、Tauri 插件和 WebView2 安装器下载，避免后续打包重复下载；Tauri 继续负责工具版本及已有校验逻辑。
- CI 专用配置只关闭重复的 `beforeBuildCommand`，使用前先检查本次验证生成的五个 HTML 入口（包含快捷提问）；本地 `pnpm package:windows` 仍自动构建前端。TypeScript 检查、前端测试、前端构建、Rust 测试、打包、产物校验和 release 启动检查分别计时并写入 Actions 运行摘要，便于比较缓存命中后的各阶段耗时。
- 打包后实际启动 release 程序，要求 20 秒内写入当前进程的 `ready` 日志且保持运行；失败时不上传安装包。`startup.log` 仅保留在运行机器本地。
- 复用同一 release 程序生成白名单便携 ZIP；先验证配置导入、ZIP 内容及 Scoop 清单生成，再实际解压并通过目录联接模拟两次 Scoop 版本启动。配置和日志必须保留，WebView 必须写入程序旁的 webview 且不跨版本共享，安装版启动日志必须保持不变。便携文件、清单、三个附件的 SHA-256 汇总到 `bundle/distribution` 后上传。
- `popper.json` 在构建时使用实际 ZIP 的 SHA-256 和 `GITHUB_REPOSITORY` 生成；通过 Release 的 `releases/latest/download/popper.json` URL 提供 Scoop 安装和更新。构建任务不修改仓库清单或推送提交，权限仍为只读。
- 首次构建、缓存被清理或工具链升级时仍需冷编译；后续缓存命中时才有主要收益，具体节省时间需比较云端运行。GitHub 的系统镜像和 NASM 软件包会更新，因此不承诺逐字节可复现。

安装包仍是未签名测试包，可能触发 SmartScreen。云端单元测试不能替代 Zotero、Chrome 等桌面应用中的真实划词、快捷键和剪贴板回归。

此配置需推送后完成首次云端运行验证；本地语法检查不等同于 CI 已通过。

参考：[手动触发工作流](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)、[复用工作流](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows)、[GitHub CLI 创建 Release](https://cli.github.com/manual/gh_release_create)、[Windows runner 环境](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md)、[Rust 缓存策略](https://github.com/Swatinem/rust-cache)、[Cargo 测试目标](https://doc.rust-lang.org/cargo/commands/cargo-test.html)、[Cargo 编译配置](https://doc.rust-lang.org/cargo/reference/profiles.html)、[GitHub Actions 缓存](https://github.com/actions/cache)。
