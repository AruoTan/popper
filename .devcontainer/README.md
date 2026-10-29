# 开发容器

[开发手册](../docs/development.md) · [Windows 构建与发布](../docs/build-and-release.md)

## 启动

安装 Docker 与 VS Code Dev Containers 扩展，打开完整仓库，执行 **Dev Containers: Reopen in Container**。Windows 的 Docker 使用 Linux containers。首次创建需联网，初始化按锁文件安装依赖；失败后执行 `bash .devcontainer/setup.sh` 重试。

验证前端：

```sh
bash .devcontainer/verify.sh
```

启动预览：

```sh
pnpm dev:web --host 0.0.0.0
```

编辑器转发 1420 端口，打开 `http://localhost:1420/settings/index.html`。浏览器不包含 Tauri IPC，不能验收读取设置、模型调用、屏幕捕捉或系统窗口操作。

## 环境

容器使用 Debian Bookworm slim、Node 22.22.0、pnpm 11.7.0、Rust 1.97.1、rustfmt、rust-src 和 Git。源码挂载到 `/workspaces/popper`，使用 `node` 用户；不安装平台原生编译依赖。

| 数据 | 位置 |
| --- | --- |
| 依赖 | 项目专属 `node_modules` 命名卷 |
| pnpm store | `/home/node/.cache/pnpm-store` |
| Cargo target | `/home/node/.cache/cargo-target` |
| 通用缓存 | `dev-cache` 命名卷 |

rust-analyzer 关闭保存检查、构建脚本和过程宏，仅用于源码导航。初始化将 GitHub SSH 连接配置为 `ssh.github.com:443`。编辑器的凭据转发由编辑器配置控制。

## 验证范围

容器执行 TypeScript 检查、前端测试和网页构建，支持 Rust 编辑与格式检查：

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

Windows Rust 测试与打包使用手动 Actions 或 Windows 原生环境；macOS 构建与桌面操作需在 Mac 上完成。容器不能运行 Tauri 桌面应用，前端通过不代表原生检查通过。

## 维护

修改 Dockerfile 后执行 **Dev Containers: Rebuild Container**。更新 `packageManager` 时同步 `PNPM_VERSION`，更新 Rust 或 Node 时同步 Windows 工作流。迁移源码保留容器配置和锁文件，不复制依赖或编译目录。

Windows 文件监听不及时，可将仓库放到 WSL2 Linux 文件系统，或使用 `CHOKIDAR_USEPOLLING=true pnpm dev:web --host 0.0.0.0`。删除缓存前停止容器，并核对具体卷名，避免影响源码或其他项目。
