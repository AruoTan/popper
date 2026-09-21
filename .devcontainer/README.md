# Popper 独立开发容器

用于前端开发和 Rust 源码编辑；Windows Rust 编译、测试和安装包构建交给 GitHub Actions。不读取宿主机的 Node、Rust、Windows SDK、用户设置或 API Key，不挂载 Docker socket。

## 开始使用

1. 安装并启动 Docker（Windows 使用 Docker Desktop 的 Linux containers / WSL2 后端），在 VS Code 中安装 Dev Containers 扩展。
2. 克隆或解压完整项目，打开项目根目录，执行 **Dev Containers: Reopen in Container**。
3. 首次构建需要联网下载镜像、Debian 软件包、Rust 和 npm 依赖；初始化自动执行 `pnpm install --frozen-lockfile`，失败后可重跑 `bash .devcontainer/setup.sh`。
4. 在容器终端执行：

```bash
# 类型检查、前端测试及构建（不执行 Rust 编译测试）
bash .devcontainer/verify.sh

# 仅开发前端，编辑器自动转发 1420 端口
pnpm dev:web --host 0.0.0.0

# 可选：检查 Rust 格式，不需要原生编译依赖
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

浏览器打开 `http://localhost:1420/settings/index.html`（也可访问 `toolbar/index.html`、`result/index.html`）。这是前端预览，不包含 Tauri IPC；读取设置、翻译、原生窗口控制等依赖后端的操作不能据此验收。默认不公开发布端口，转发由编辑器管理。

## 环境与隔离

- Debian Bookworm slim、Node 22.22.0、pnpm 11.7.0、Rust 1.97.1（含 rustfmt 和用于标准库导航的 rust-src）、Git 和 SSH 客户端。不安装 Tauri Linux 开发库、C/C++ 编译器、CMake、NASM、虚拟显示或 D-Bus。
- rust-analyzer 保留源码分析，但关闭保存时 Cargo 检查、构建脚本执行和过程宏展开，避免编辑时触发原生编译。补全和诊断并不完整，尤其是宏生成代码及 Windows 条件编译代码；不能代替 CI。配置含义见 [rust-analyzer 官方说明](https://rust-analyzer.github.io/book/configuration)。
- Node 镜像标签、Rust 和 pnpm 版本显式固定；npm/Cargo 使用仓库锁文件。Debian 软件包与镜像标签不是不可变快照，不承诺逐字节复现或离线首次构建。
- 源码绑定到 `/workspaces/popper`，没有本机盘符或用户名依赖，面向 amd64/arm64 Linux 容器；不同架构各自构建，不共享二进制缓存。
- `node_modules` 使用容器专属命名卷，遮盖宿主依赖；pnpm store 与 Cargo target 放在独立缓存卷，不使用宿主 `src-tauri/target`。卷名包含容器标识，避免不同工作区相互污染。
- 以非 root 的 `node` 用户开发；初始化仅对两个卷根目录修正权限。Rust 工具链在镜像内，Cargo 下载缓存在容器用户目录，重建后可重新下载。
- 宿主只需 Docker 和编辑器，无需安装 Node、pnpm 或 Cargo。配置不自动注入密钥；编辑器自身的 Git/SSH 凭据转发仍由编辑器设置控制。

## 平台边界

| 工作                                       | 此容器                                        |
| ------------------------------------------ | --------------------------------------------- |
| React/TypeScript 编辑、测试、构建          | 支持                                          |
| Rust 编辑、格式检查、基础源码导航          | 支持；不提供完整编译诊断                      |
| Rust 编译检查、测试                        | 容器不执行；Windows 检查和测试交给 GitHub Actions |
| UIA、受控 Ctrl+C、全局钩子、跨应用划词回归 | 必须在 Windows 原生环境验证                   |
| macOS 辅助功能与桌面回归                   | 必须在 macOS 原生环境验证                     |
| Windows NSIS 安装包                       | GitHub Actions Windows runner 构建           |
| macOS DMG 安装包                          | 仍需 macOS 原生环境，当前 Windows CI 不覆盖   |

不要在容器内用 `pnpm dev`、`pnpm verify` 或 `pnpm package:windows` 代替上述命令：原有脚本显式面向 macOS/Windows。此轻量容器也不支持直接运行 Tauri 的 Linux `cargo check`、`cargo test` 或桌面应用，因为未安装其系统编译依赖。

不希望在 Windows 主系统安装编译工具时，使用 [GitHub Actions 云端打包](../.github/README.md)：容器内开发并推送代码，在 GitHub 手动运行工作流，再下载安装包。本机只承担真实桌面交互回归。

当前工作流仅手动触发，推送或 PR 不会自动执行检查。修改 Rust 后应手动运行工作流确认 `pnpm verify:windows` 和打包结果；本地前端验证通过不代表原生代码通过。

如选择本地 Windows 打包，继续使用 `pnpm verify:windows` 和 `pnpm package:windows`，需 Visual Studio C++ Build Tools、Windows SDK、WebView2、Node/pnpm 和 Rust MSVC 工具链。macOS 继续使用项目原有命令及 Xcode Command Line Tools。

## 迁移和维护

轻量配置需要重建容器后生效；尚未实测此镜像构建和容器内完整验证。首次启动后请运行 `bash .devcontainer/verify.sh` 确认结果。

- 将源码、`.devcontainer`、`package.json`、`pnpm-workspace.yaml`、两个锁文件一并提交或复制即可；不迁移 `node_modules`、`.pnpm-store`、`target` 或本机密钥。`pnpm export:dev-source` 也包含此配置。
- 修改 Dockerfile 后执行 **Dev Containers: Rebuild Container**。升级 `packageManager` 时同步修改 `PNPM_VERSION`，初始化会检查版本一致性。
- 缓存不是源码，可停止容器后通过 Docker Desktop 精确选择本项目的 `popper-node-modules-*` / `popper-cache-*` 卷删除以重新安装；不要执行全局 volume prune。删除这些缓存不会删除绑定的源码。
- Windows 文件监听不及时可尝试 `CHOKIDAR_USEPOLLING=true pnpm dev:web --host 0.0.0.0`，或把仓库放到 WSL2 Linux 文件系统。`.gitattributes` 确保容器脚本使用 LF。
- 配置参考 [Dev Container 规范](https://containers.dev/implementors/json_reference/)。
