# Windows 便携版与 Scoop

Windows Release 同时提供安装包、`Popper_<version>_x64-portable.zip`、Scoop 清单 `popper.json` 和 `SHA256SUMS.txt`。这些附件由现有手动 Release Action 从同一提交生成。

## 使用便携版

将整个 ZIP 解压到可写目录，双击 `popper.exe`。程序旁的 `portable.flag` 启用便携模式，首次启动创建：

```text
Popper/
├─ popper.exe
├─ portable.flag
├─ Import-InstalledSettings.ps1
└─ data/
   ├─ config/
   │  ├─ settings.json
   │  ├─ api-keys.enc.json
   │  └─ .api-keys.key
   ├─ logs/
   └─ webview/
```

设置、服务商密钥、欧路授权、启动日志、划词诊断和所有窗口的 WebView 数据均使用此目录。便携模式不自动导入安装版配置。目录不可写会显示错误并停止启动。

需要系统已安装 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。ZIP 不包含 Runtime；系统运行库、Windows 自身的临时文件和系统诊断不属于 Popper 的配置目录。

手动升级时先退出 Popper，将新版解压到新目录，然后把旧版的 `data` 移到新版程序旁。发布 ZIP 只包含程序、便携标记、许可与说明文件，不包含用户数据。

## Scoop 安装与升级

首次发布带有 `popper.json` 的正式 Release 后执行：

```powershell
scoop install https://github.com/AruoTan/popper/releases/latest/download/popper.json
```

之后更新：

```powershell
# 先从托盘退出 Popper；禁用托盘时可在任务管理器结束 Popper。
scoop update
scoop update popper
```

清单使用 `persist: "data"`，Scoop 创建目录联接：

```text
<Scoop>/apps/popper/current/data → <Scoop>/persist/popper/data
```

程序从自己的目录访问 `data`，无需设置环境变量，也不假定 Scoop 安装在用户主目录。开始菜单快捷方式和 `popper` 命令均由 Scoop 创建。升级、卸载前的清单检查要求 Popper 已退出。

通过 `releases/latest/download/popper.json` 安装时，Scoop 更新读取同一清单 URL，跟随最新正式版。预发布版可从对应 Release 的 `popper.json` 附件 URL 安装；固定版本 URL 会继续跟踪该固定清单，需显式切换 URL 才能跟随其他版本。草稿附件须先发布才能公开安装。

普通 `scoop uninstall popper` 保留持久化数据；`scoop uninstall -p popper` 同时清除它。清单也可放入自己的 bucket，内含版本检测和基于 `SHA256SUMS.txt` 的自动更新规则。项目目前通过 Release 附件提供清单，无需维护额外的 bucket 仓库。

## 从安装版迁移

先退出所有 Popper 实例，在便携版首次启动前运行 ZIP 中的导入脚本：

```powershell
& .\Import-InstalledSettings.ps1
```

Scoop 安装后可运行：

```powershell
$popperDirectory = scoop prefix popper
& (Join-Path $popperDirectory 'Import-InstalledSettings.ps1')
```

脚本从 `%APPDATA%\com.local.popper` 复制 `settings.json` 及本地密钥文件，完整复制后才发布 `data/config`，不修改源配置。服务商密钥和欧路授权使用原有加密格式，无需重新填写。目标 `config` 已存在时拒绝覆盖；如已启动过便携版，先退出并备份、移走该目录再导入。备份 `config` 时应妥善保管其中的 `.api-keys.key`，它用于解密密钥文件。

也可以指定源目录和目标数据根目录：

```powershell
& .\Import-InstalledSettings.ps1 -SourceDirectory 'D:\Old Popper Config' -DataDirectory 'D:\Popper Data'
```

## 自定义目录与运行模式

```powershell
.\popper.exe --data-dir 'D:\Popper Data'
```

`--data-dir` 接受绝对路径，优先于便携标记；支持 `--data-dir=<绝对路径>`。指定的是数据根目录，程序在其下使用 `config`、`logs` 和 `webview`。划词辅助进程继承同一目录。

没有参数和便携标记时，安装版沿用 Tauri 的原配置目录及 `%LOCALAPPDATA%\Popper` 日志目录，macOS 路径不变。Windows 安装版与便携版共用单实例机制；同一数据目录的再次启动打开设置，不同数据目录的启动会提示先退出原实例，防止误用另一个配置。切换运行模式或数据目录前，应先退出正在运行的实例。

## 开发与验证

```powershell
pnpm build:windows
pnpm verify:artifacts:windows
pnpm package:portable:windows
node apps/windows/scripts/prepare-release-assets.mjs
```

便携 ZIP 位于 `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/portable`，完整发布附件位于相邻的 `distribution`。设置 `CARGO_TARGET_DIR` 时，本地便携打包与发布附件生成脚本使用该目录。`GITHUB_REPOSITORY` 可指定生成清单的目标仓库，默认 `AruoTan/popper`。

CI 运行路径、迁移、密钥迁移、ZIP 白名单及校验值测试，并从含空格和中文的临时目录实际启动 ZIP 程序，通过目录联接模拟 Scoop 切换两个版本目录，确认设置保留、WebView 使用便携目录，且未写入安装版启动日志。

参考：[Scoop 持久化数据](https://github.com/ScoopInstaller/Scoop/wiki/Persistent-data)、[Scoop 清单](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests)。
