# Popper

[English](./README.en.md) | **中文**

Popper 是桌面划词与 AI 助手，支持翻译、查词、总结、解释、润色和多轮提问。Windows 版还提供截图、标注、手动滚动截图和浮动贴图。

[下载安装](https://github.com/AruoTan/popper/releases/latest) · [使用指南](./USER_GUIDE.md) · [开发手册](./DEVELOPMENT_HANDBOOK.md)

## 功能

- **划词操作**：选中文字后长按右键，调用 AI 动作、搜索或复制；支持自定义动作、提示词和模型。
- **快捷提问**：通过快捷键打开独立输入框，附加图片、文档或文本，在结果窗口继续对话。
- **翻译与查词**：多语言翻译、英文词典、发音和欧路生词本收藏。
- **屏幕捕捉**：窗口与控件吸附、区域截图、六种标注、手动上下滚动拼接、取色和浮动贴图。
- **模型接入**：支持多个 OpenAI 兼容服务和 DSH Docker，流式显示正文与推理，支持切换模型。

## 安装

| 平台 | 系统要求 | 安装方式 |
| --- | --- | --- |
| Windows | Windows 10/11 x64；滚动截图需要 Windows 10 2004+ | 安装包、便携 ZIP、Scoop |
| macOS | macOS 12+，Apple Silicon | DMG；不含屏幕捕捉功能 |

从 [Releases](https://github.com/AruoTan/popper/releases) 下载对应产物。Windows 安装包按需安装 WebView2；便携版和 Scoop 需系统已有 WebView2。macOS 将应用拖入“应用程序”，首次运行授予辅助功能权限。

Scoop 安装：

```powershell
scoop install https://github.com/AruoTan/popper/releases/latest/download/popper.json
```

当前产物未签名；macOS 产物未公证。安装与升级步骤见[使用指南](./USER_GUIDE.md#安装与升级)，便携版数据管理见 [Windows 便携版与 Scoop](./WINDOWS_PORTABLE.md)。

## 开始使用

1. 从托盘或菜单栏打开设置，在“通用”中添加 AI 服务商、获取模型并保存，在“动作”中选择默认模型。
2. 在任意应用选中文字，长按右键 250 毫秒，点击所需动作。
3. 按 `Ctrl+Shift+Space` 打开快捷提问；Windows 按 `PrtSc` 开始截图。

API Key 在本地加密保存。Popper 不保存划词历史；查词、AI 请求和收藏按用户操作向相应服务发送内容。数据范围见[隐私与数据](./USER_GUIDE.md#隐私与数据)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [使用指南](./USER_GUIDE.md) | 安装、模型配置、划词、附件、截图与贴图 |
| [Windows 便携版与 Scoop](./WINDOWS_PORTABLE.md) | 数据目录、导入、升级与卸载 |
| [DSH Docker 接入](./DSH_CONNECTION.md) | 网关配置、会话与连接排查 |
| [开发手册](./DEVELOPMENT_HANDBOOK.md) | 环境、架构、验证、版本与源码导出 |
| [屏幕捕捉开发](./SCREEN_CAPTURE.md) | 会话、平台边界、资源管理与验收 |
| [Windows 构建与发布](./.github/BUILD_AND_RELEASE.md) | Actions 构建、发布草稿与产物检查 |

## 许可证

Popper 使用 [GPL-3.0-only](./LICENSE)。第三方归属与许可见 [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)、[LICENSE.selection-hook](./LICENSE.selection-hook) 和 [LICENSE.capture](./LICENSE.capture)。
