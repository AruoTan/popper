# Windows 云端打包

本地在 Dev Container 开发，GitHub 托管的 `windows-2022` runner 完成 Windows 检查和打包。本机不需要安装 MSVC、Windows SDK、Rust 或 Node。

## 使用

1. 将源码和 `.github` 配置提交、推送到 GitHub 仓库。工作流首次必须存在于默认分支，Actions 页面才会显示手动运行按钮。
2. 打开 **Actions → Build Windows installer → Run workflow**，选择待构建分支并运行。
3. 成功后，在该次运行的 **Artifacts** 下载 `Popper-windows-x64-运行编号`，解压得到 `.exe` 和 `SHA256SUMS.txt`。
4. 可用 PowerShell 的 `Get-FileHash .\Popper_0.5.1_x64-setup.exe -Algorithm SHA256` 与校验文件比对（文件名随版本变化）。

工作流仅手动触发，不因 push/PR 自动消耗构建额度，不创建 Release、不推送提交、不需要发布令牌或签名密钥。产物保留 14 天；请及时下载。私有仓库需留意账户的 Actions 分钟数、存储额度及组织策略。

## 流程与维护

- Node 22.22.0、Rust 1.97.1 MSVC，与开发容器版本保持一致；pnpm 精确版本从 `package.json` 读取。
- 通过 `pnpm install --frozen-lockfile` 安装依赖，自动发现 runner 的 Visual Studio/Windows SDK；安装并优先使用 NASM，避免 Strawberry Perl 同名工具干扰。
- 运行 `pnpm verify:windows`，成功后运行 `pnpm package:windows`。沿用已有 PE 架构、版本及 SHA-256 校验，不跳过测试。
- 任一步骤失败都会停止；仅成功产物上传。任务最多运行 60 分钟，同一分支的并发构建不会相互中断。
- 第三方 Actions 固定到提交 SHA，仓库权限只有 `contents: read`。升级 Actions 时同步核对官方版本与 SHA。
- 当前不缓存 Cargo 编译目录，优先保证干净构建；冷构建较慢。GitHub 的系统镜像和 NASM 软件包会更新，因此不承诺逐字节可复现。

安装包仍是未签名测试包，可能触发 SmartScreen。云端单元测试不能替代 Zotero、Chrome 等桌面应用中的真实划词、快捷键和剪贴板回归。

此配置需推送后完成首次云端运行验证；本地语法检查不等同于 CI 已通过。

参考：[手动触发工作流](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)、[Windows runner 环境](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md)。
