# macOS 平台

[开发手册](../../DEVELOPMENT_HANDBOOK.md) · [使用指南](../../USER_GUIDE.md#安装与升级)

| 路径 | 职责 |
| --- | --- |
| `native/selection_bridge.h`、`native/selection_bridge.mm` | Accessibility／AppKit 选区桥接 |
| `icons/icon.icns`、`icons/tray-template.png` | 应用与菜单栏图标 |
| `scripts/verify-artifacts.mjs` | App／DMG 架构、版本、资源与路径检查 |

`src-tauri/build.rs` 编译原生桥接，Tauri 配置引用应用图标，运行时加载菜单栏模板图标。开发目标为 `aarch64-apple-darwin`，需要辅助功能权限；不包含屏幕捕捉入口。验证与打包命令统一见开发手册。
