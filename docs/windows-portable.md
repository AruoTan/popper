# Windows 便携版与 Scoop

[使用指南](./user-guide.md) · [开发手册](./development.md)

## 便携版

从 [Releases](https://github.com/AruoTan/popper/releases) 下载 `Popper_<version>_x64-portable.zip`，完整解压到可写目录并运行 `popper.exe`。保留同目录的 `portable.flag`，并先安装 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。

| 路径 | 内容 |
| --- | --- |
| `config/settings.json` | 当前版本 18 的设置 |
| `config/api-keys.enc.json`、`config/.api-keys.key` | 加密凭据与本地解密文件 |
| `logs/` | 启动和诊断日志 |
| `webview/` | 当前程序版本的 WebView 数据 |

首次启动自动创建目录，不自动导入安装版配置。升级前退出 Popper，将新版解压到新目录，把旧版 `config` 和 `logs` 移到新版旁；`webview` 由新版重新生成。仅支持当前配置格式，旧配置需备份后重新设置。

## Scoop

安装：

```powershell
scoop install https://github.com/AruoTan/popper/releases/latest/download/popper.json
```

退出 Popper 后更新：

```powershell
scoop update
scoop update popper
```

Scoop 将 `config` 和 `logs` 联接到 `persist/popper/`，`webview` 留在各版本程序目录，不跨版本复用。普通卸载保留持久化数据，`scoop uninstall -p popper` 同时删除数据。升级与卸载检查要求 Popper 已退出。

最新正式版清单 URL 跟随正式发布。安装特定版本或预发布版时使用对应 Release 的 `popper.json` URL，此固定地址不会自动切换到其他版本。

## 导入安装版配置

导入仅支持版本 18。先退出全部 Popper 实例，再在便携目录执行：

```powershell
& .\Import-InstalledSettings.ps1
```

脚本从 `%APPDATA%\com.local.popper` 复制设置与密钥，不修改源文件，不覆盖已存在的目标 `config`。已运行过便携版时，先备份并移走目标 `config`。密钥文件与 `.api-keys.key` 必须一起保留。

Scoop 用户执行：

```powershell
$popperDirectory = scoop prefix popper
& (Join-Path $popperDirectory 'Import-InstalledSettings.ps1')
```

指定源配置目录与目标数据根目录：

```powershell
& .\Import-InstalledSettings.ps1 -SourceDirectory 'D:\Old Popper Config' -DataDirectory 'D:\Popper Data'
```

## 自定义数据目录

```powershell
.\popper.exe --data-dir 'D:\Popper Data'
```

参数必须是绝对路径，优先于 `portable.flag`，其下使用 `config`、`logs` 和 `webview`。安装版与便携版共用单实例机制；切换模式或数据目录前先退出原实例。同目录再次启动打开设置，不同目录再次启动会提示退出。

未指定参数且没有便携标记时，安装版配置在 `%APPDATA%\com.local.popper`，日志在 `%LOCALAPPDATA%\Popper`。
