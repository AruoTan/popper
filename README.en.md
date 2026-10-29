# Popper

**English** | [中文](./README.md)

Popper is a desktop text selection and AI assistant for translation, dictionary lookup, summaries, explanations, rewriting, and follow-up questions. On Windows, it also provides screen capture, annotations, manual scrolling screenshots, and floating images.

[Download](https://github.com/AruoTan/popper/releases/latest) · [User guide](./USER_GUIDE.md) · [Development handbook](./DEVELOPMENT_HANDBOOK.md)

## Features

- **Selection actions**: select text and hold the right mouse button to run AI actions, search, or copy. Customize actions, prompts, and models.
- **Quick Ask**: open a standalone composer with a shortcut, attach images, documents, or text, and continue the conversation in a result window.
- **Translation and dictionary**: multilingual translation, English dictionary lookup, pronunciation, and saving words to Eudic.
- **Screen capture**: window and control snapping, region capture, six annotation tools, manual vertical scrolling capture, color picking, and floating images.
- **Model providers**: connect multiple OpenAI-compatible services or DSH Docker, stream text and reasoning, and switch models.

## Installation

| Platform | Requirements | Packages |
| --- | --- | --- |
| Windows | Windows 10/11 x64; scrolling capture requires Windows 10 2004+ | Installer, portable ZIP, Scoop |
| macOS | macOS 12+, Apple Silicon | DMG; screen capture is unavailable |

Download a package from [Releases](https://github.com/AruoTan/popper/releases). The Windows installer installs WebView2 when needed; portable and Scoop installations require it to be installed separately. On macOS, drag Popper into Applications and grant Accessibility permission on first launch.

Install with Scoop:

```powershell
scoop install https://github.com/AruoTan/popper/releases/latest/download/popper.json
```

Current packages are unsigned; macOS packages are also unnotarized. See the [user guide](./USER_GUIDE.md#安装与升级) for installation and upgrades and the [portable guide](./WINDOWS_PORTABLE.md) for Windows data management.

## Getting started

1. Open settings from the tray or menu bar. Add a provider under General, fetch and save its models, then select the default model under Actions.
2. Select text in another application, hold the right mouse button for 250 ms, and choose an action.
3. Press `Ctrl+Shift+Space` for Quick Ask. On Windows, press `PrtSc` for screen capture.

API keys are encrypted locally. Popper does not save selection history. Dictionary queries, AI requests, and vocabulary saves send content to the corresponding service when triggered. See [privacy and data](./USER_GUIDE.md#隐私与数据) for details.

## Documentation

The following guides are maintained in Chinese.

| Document | Contents |
| --- | --- |
| [User guide](./USER_GUIDE.md) | Installation, providers, selection actions, attachments, capture, and floating images |
| [Windows portable and Scoop](./WINDOWS_PORTABLE.md) | Data directories, importing settings, upgrades, and uninstalling |
| [DSH Docker](./DSH_CONNECTION.md) | Gateway setup, sessions, and connection troubleshooting |
| [Development handbook](./DEVELOPMENT_HANDBOOK.md) | Setup, architecture, checks, versioning, and source exports |
| [Screen capture development](./SCREEN_CAPTURE.md) | Sessions, platform boundaries, resource management, and acceptance checks |
| [Windows builds and releases](./.github/BUILD_AND_RELEASE.md) | Actions builds, release drafts, and artifact verification |

## License

Popper uses [GPL-3.0-only](./LICENSE). Third-party attribution and licenses are provided in [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md), [LICENSE.selection-hook](./LICENSE.selection-hook), and [LICENSE.capture](./LICENSE.capture).
