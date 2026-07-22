# 可迁移开发源码包

这是 TextLens 0.3.53 的干净源码工作区，只包含继续开发所需的源代码、配置和文档，不包含已编译应用或安装包。

## 包含

- 共享前端源码：`src/`
- 平台代码与资源：`apps/`、`assets/`
- Rust/Tauri 后端：`src-tauri/`（不含 target 编译产物）
- 构建脚本、锁文件、配置、README、开发历史、许可证与第三方说明

## 不包含

- 已编译应用、安装包和 `release/` 发布快照
- 前端构建产物 `dist/`
- 依赖目录 `node_modules/`
- Rust 编译目录 `src-tauri/target/`
- 临时诊断、归档目录、缓存和 `.tsbuildinfo` 文件

## 在新机器上继续开发

环境要求：

- Node.js 22+
- pnpm 11+
- Rust 稳定版
- Windows：MSVC 工具链、WebView2
- macOS：Xcode Command Line Tools（Apple Silicon 优先）

安装依赖：

    pnpm install

验证：

    # Windows
    pnpm verify:windows

    # Apple Silicon macOS
    pnpm verify

常用命令：

    # Windows 开发
    pnpm dev:windows

    # Windows 打包安装包
    pnpm package:windows

    # macOS 开发 / 打包
    pnpm dev
    pnpm package

再次导出干净源码：

    pnpm export:dev-source

说明：默认只预置 OpenAI Base URL，API Key 为空；不要把本机密钥、`.env` 或私有配置复制进源码包。
