# Codex Monitor

[English](../../README.md) | 简体中文 | [繁體中文](README.zh-TW.md) | [日本語](README.ja.md)

[![GitHub release](https://img.shields.io/github/v/release/koodev24/codex-monitor)](https://github.com/koodev24/codex-monitor/releases/latest)
[![Release build](https://img.shields.io/github/actions/workflow/status/koodev24/codex-monitor/release-v2-tauri.yml?label=release%20build)](https://github.com/koodev24/codex-monitor/actions)
![macOS](https://img.shields.io/badge/macOS-supported-blue)
![Windows](https://img.shields.io/badge/Windows-supported-blue)
![Linux](https://img.shields.io/badge/Linux-supported-blue)

原生桌面应用，用于查看所有 ChatGPT 账号的 Codex API 用量：5 小时与每周配额、重置倒计时、每个账号的重置额度、自动获取，以及带签名的应用内更新。

## 下载

从 [Releases 页面](https://github.com/koodev24/codex-monitor/releases/latest) 获取最新版本。各平台的发布文件名格式一致：

| 系统 | 架构 | 文件 |
| --- | --- | --- |
| macOS | Apple Silicon | `CodexMonitor-<version>-macos-arm64.dmg` |
| macOS | Intel | `CodexMonitor-<version>-macos-x64.dmg` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64-setup.exe` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64.msi` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.AppImage` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.deb` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.rpm` |

每个版本还包含：

- `latest.json`：应用读取的更新清单，列出各平台带签名的更新文件。更新端点要求固定名称，因此文件名不含版本号。请勿手动修改或重新发布。
- `CodexMonitor-<version>-SHA256SUMS.txt`：可下载文件的校验和。
- `CodexMonitor-<version>-macos-*.app.tar.gz` 与 `.sig` 文件：更新载荷和签名。macOS 正常安装请使用 DMG。

使用校验和文件验证下载：

```bash
shasum -a 256 -c CodexMonitor-<version>-SHA256SUMS.txt
```

### macOS 首次启动

应用未签名，macOS 会阻止首次启动。右键点击应用并选择 **打开**，或清除一次隔离标记：

```bash
xattr -dr com.apple.quarantine /Applications/CodexMonitor.app
open /Applications/CodexMonitor.app
```

请将应用放在 `/Applications` 或 `~/Applications`。应用内更新会原地替换应用包。

### 从 v1 Python 应用迁移

v2.6.0 起不再发布旧 Python 更新器依赖的 `CodexMonitor-macOS.zip` 桥接包。仍在使用 v1 时，请手动迁移：

1. 退出旧应用。
2. 从最新版本下载适合你 Mac 的 DMG（Apple Silicon 或 Intel）。
3. 打开 DMG，将 **CodexMonitor.app** 移到 `/Applications` 或 `~/Applications`。
4. 启动新应用。首次启动时，如果存在旧数据目录，会从中导入已保存的账号和用量历史；旧文件只读取，不会删除。
5. 如果账号缺失，请使用 **添加账号** 登录或导入保存的 `auth.json`。之后可以删除旧的 Python 应用。

## 功能

- **多账号配额**：每个账号的 5 小时与每周剩余配额及重置倒计时。
- **重置额度**：每个账号的额度记录，含到期时间与剩余时间。
- **获取用量**：一键获取单个账号或全部账号的配额，并可配置自动获取间隔。
- **账号管理**：通过 Codex 登录添加账号、归档不用的账号，并安全切换当前账号：Codex 会关闭、保留最新令牌、写入所选会话，然后自动重新打开。
- **菜单栏弹窗（macOS）**：菜单栏标题显示当前账号的 5 小时与每周配额；点击可查看紧凑账号列表、一步切换并打开主界面。
- **实时日志**：固定高度的活动面板，支持一键复制和清空。
- **设置**：界面语言、菜单栏显示、桌面通知和自动检查更新，修改后立即保存。
- **应用内更新**：签名更新源，支持手动检查，并每 6 小时自动检查。

## 语言

界面支持英语、简体中文、繁体中文和日语。在 **设置 → 语言** 中选择，或保留 **系统** 跟随操作系统语言。菜单栏原生菜单和主界面使用同一设置，应用状态消息也会跟随该设置。后端诊断日志保持英语，便于在问题报告中排查。

## 使用

1. 启动应用，点击 **添加账号**，通过 Codex 登录。
2. 点击 **刷新配额**（或等待自动获取）获取当前账号用量；点击 **获取所有账号** 获取全部备用账号。
3. 将鼠标悬停在工具栏按钮上可查看说明；点击 **眼睛** 切换归档账号，点击 **太阳/月亮** 切换明暗主题，点击 **齿轮** 打开设置。
4. 点击底部的 **活动** 展开或收起日志。切换账号只需确认一次，关闭 Codex、切换凭据和重新打开会作为一步完成。
5. 在 macOS 上，关闭窗口后 Codex Monitor 会继续在菜单栏运行。使用托盘弹窗重新打开主界面，或从菜单选择 **退出**。如果在设置中关闭 **显示菜单栏图标**，关闭窗口将直接退出应用。

## 更新机制

- **云下载** 按钮会检查最新版本中的 `latest.json`。若有更新，会下载对应系统和架构的安装包，验证签名，安装并重新启动。
- 启动时以及每 6 小时还会静默检查。结果显示在状态栏，不弹出窗口。
- 所有平台构建完成后，CI 会重新生成 `latest.json`，确保它始终指向同一版本的资源。因此已发布的版本不会手动移动或修改。

## 开发

需要 Node 22 和稳定版 Rust 工具链。

```bash
cd apps/desktop
npm install
npm run tauri dev
```

常用命令（除注明外均在 `apps/desktop` 下运行）：

```bash
npm run build          # 前端类型检查（tsc）+ Vite 构建
npm test               # 前端单元测试
npm run tauri dev      # 带热更新的完整应用
npm run tauri build    # 本地发布包（不会提交）
cargo test             # Rust 后端测试（apps/desktop/src-tauri）
cargo clippy --all-targets -- -D warnings
```

项目结构：

```text
apps/desktop/
  src/                 # React 前端（组件、状态、i18n、格式化）
  src-tauri/           # Rust 后端（API、存储、监听、更新）
scripts/
  set-version.mjs      # CI 中根据发布标签同步版本
  finalize-release.mjs # 规范化发布资源并生成 latest.json
  release-notes.mjs    # 从 CHANGELOG.md 提取发布说明
```

## 发布

推送 `v2*` 标签。CI 会创建草稿版本，构建 macOS arm64、macOS x64、Windows x64 和 Linux x64，然后统一资源名称、重新生成 `latest.json`、写入 `CodexMonitor-<version>-SHA256SUMS.txt`，最后一次性发布：

```bash
git tag v2.6.0
git push origin v2.6.0
```

发布规则：

- **只发布 `v2*` 标签。** 此工作流只对 `v2*` 运行；不再发布 v1 版本。
- 已发布的版本不会移动或覆盖。如需修复，请发布新的补丁版本。
- 本地构建的安装包不会提交（见 `.gitignore`）；发布包只来自 CI。
- 发布说明从 `CHANGELOG.md` 生成，请在打标签前添加对应版本章节。
