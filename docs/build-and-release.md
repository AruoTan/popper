# Windows 构建与发布

[开发手册](./development.md) · [便携版与 Scoop](./windows-portable.md)

## 构建

在 GitHub **Actions → Build Windows installer → Run workflow** 选择分支或标签。工作流仅手动触发，也可被发布工作流调用；推送和 PR 不自动运行。

成功后下载 Artifacts 中的 `Popper-windows-x64-<运行编号>`，包含安装 EXE、便携 ZIP、`popper.json` 和 `SHA256SUMS.txt`。产物保留 14 天。构建不创建 Release，不写入仓库。

## 发布

先提交并推送全部改动，再创建 `vX.Y.Z` 格式的版本标签。将命令中的 `vX.Y.Z` 换为待发布版本：

```sh
git tag -a vX.Y.Z -m "Popper vX.Y.Z"
git push origin refs/tags/vX.Y.Z
gh workflow run release-windows.yml --ref vX.Y.Z -f draft=true -f prerelease=false
```

也可在 **Actions → Release Windows installer → Run workflow** 选择标签。推送标签本身不会启动发布。工作流须先存在于默认分支，Actions 才提供手动入口。

默认创建草稿，检查附件与说明后在 Releases 页面发布。`draft=false` 在附件上传完成后直接发布；`prerelease=true` 只设置 GitHub 预发布状态，不改变标签格式。已有同名 Release，包括草稿，均拒绝覆盖。

发布要求工作区干净、当前提交恰好有一个版本标签。准备和发布阶段均核对标签与固定源码提交，附件使用本次构建结果并重新校验 SHA-256；`--verify-tag` 阻止自动创建标签。版本规则见[应用版本](./development.md#应用版本)。

## 验证链

工作流在 `windows-2022` 上使用 Node 22.22.0、Rust 1.97.1 MSVC 和 `package.json` 指定的 pnpm，获取完整 Git 历史与标签，按锁文件安装依赖。

1. 类型检查、Node 脚本测试、完整 Vitest、网页构建。
2. Windows Rust 全目标测试，串行执行测试用例。
3. 确认本次构建的六个页面齐全：设置、划词工具栏、结果、启动提示、快捷提问和截图。
4. 复用已验证的前端输出构建 NSIS，检查 x64 PE、版本、资源与校验值。
5. 实际启动 release 程序，要求同一进程在 60 秒内写入 `ready` 并保持运行；超时由启动检查脚本统一定义。
6. 生成白名单便携 ZIP，检查配置导入与密钥保留；从含中文和空格的目录启动，模拟 Scoop 跨版本持久化。
7. 生成 Scoop 清单与全部附件的 SHA-256，汇总到 `bundle/distribution/` 后上传。

任一步失败即停止，不上传不完整产物。启动日志保留在运行机器。CI 的原生测试不能替代 Windows 桌面的截图、焦点、剪贴板和多 DPI 验收。

## 维护

构建任务只有 `contents: read`，发布任务使用 `contents: write` 与自动提供的 `GITHUB_TOKEN`。Release 串行执行，不需要额外 PAT；组织策略需允许发布任务写入内容。

NASM 由 `scripts/setup-nasm.ps1` 从官方下载固定版本 ZIP，核验 SHA-256 与可执行文件版本，再加入 PATH。此脚本依赖 GitHub runner 环境。pnpm store、Cargo 依赖与目标产物、Tauri 打包工具分别缓存；应用和安装包仍由本次源码生成。

第三方 Actions 固定到提交 SHA。修改工具链时同步检查开发容器、锁文件与缓存键。构建摘要包含各阶段耗时，失败时先定位首个测试或编译错误，不只看最终 PowerShell 退出码。
