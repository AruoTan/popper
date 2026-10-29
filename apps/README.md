# 平台代码

[开发手册](../DEVELOPMENT_HANDBOOK.md#代码结构)

Popper 共用 Rust／Tauri 后端与 React 界面。`apps/` 保存平台专属源码、资源和产物脚本。

| 目录 | 内容 |
| --- | --- |
| [windows](./windows/README.md) | 划词后端、启动提示、图标、便携版和发布校验 |
| [macos](./macos/README.md) | 原生选区桥接、图标与产物校验 |

共享业务位于 `src/` 和 `src-tauri/src/`。Tauri 配置与 `build.rs` 引用平台资源；Windows 屏幕捕捉位于 `src-tauri/src/capture/`，不在此目录重复维护。
