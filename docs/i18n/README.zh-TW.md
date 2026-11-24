# Codex Monitor

[English](../../README.md) | [简体中文](README.zh-CN.md) | 繁體中文 | [日本語](README.ja.md)

[![GitHub release](https://img.shields.io/github/v/release/koodev24/codex-monitor)](https://github.com/koodev24/codex-monitor/releases/latest)
[![Release build](https://img.shields.io/github/actions/workflow/status/koodev24/codex-monitor/release-v2-tauri.yml?label=release%20build)](https://github.com/koodev24/codex-monitor/actions)
![macOS](https://img.shields.io/badge/macOS-supported-blue)
![Windows](https://img.shields.io/badge/Windows-supported-blue)
![Linux](https://img.shields.io/badge/Linux-supported-blue)

原生桌面應用程式，用於檢視所有 ChatGPT 帳號的 Codex API 用量：5 小時與每週配額、重設倒數、每個帳號的重設額度、自動擷取，以及具簽章驗證的應用程式內更新。

## 下載

從 [Releases 頁面](https://github.com/koodev24/codex-monitor/releases/latest) 取得最新版本。各平台的發布檔案名稱格式一致：

| 系統 | 架構 | 檔案 |
| --- | --- | --- |
| macOS | Apple Silicon | `CodexMonitor-<version>-macos-arm64.dmg` |
| macOS | Intel | `CodexMonitor-<version>-macos-x64.dmg` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64-setup.exe` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64.msi` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.AppImage` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.deb` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.rpm` |

每個版本還包含：

- `latest.json`：應用程式讀取的更新清單，列出各平台帶簽章的更新檔案。更新端點要求固定名稱，因此檔名不含版本號。請勿手動修改或重新發布。
- `CodexMonitor-<version>-SHA256SUMS.txt`：可下載檔案的校驗碼。
- `CodexMonitor-<version>-macos-*.app.tar.gz` 與 `.sig` 檔案：更新承載檔和簽章。macOS 正常安裝請使用 DMG。

使用校驗碼檔案驗證下載：

```bash
shasum -a 256 -c CodexMonitor-<version>-SHA256SUMS.txt
```

### macOS 首次啟動

應用程式未簽章，macOS 會阻止首次啟動。以滑鼠右鍵點按應用程式並選擇 **打開**，或清除一次隔離標記：

```bash
xattr -dr com.apple.quarantine /Applications/CodexMonitor.app
open /Applications/CodexMonitor.app
```

請將應用程式放在 `/Applications` 或 `~/Applications`。應用程式內更新會就地替換應用程式套件。

### 從 v1 Python 應用程式遷移

v2.6.0 起不再發布舊 Python 更新器依賴的 `CodexMonitor-macOS.zip` 橋接檔。仍在使用 v1 時，請手動遷移：

1. 結束舊應用程式。
2. 從最新版本下載適合你 Mac 的 DMG（Apple Silicon 或 Intel）。
3. 開啟 DMG，將 **CodexMonitor.app** 移到 `/Applications` 或 `~/Applications`。
4. 啟動新應用程式。首次啟動時，若存在舊資料目錄，會從中匯入已儲存的帳號與用量記錄；舊檔案只會讀取，不會刪除。
5. 若帳號缺失，請使用 **新增帳號** 登入或匯入已儲存的 `auth.json`。之後可以刪除舊的 Python 應用程式。

## 功能

- **多帳號配額**：每個帳號的 5 小時與每週剩餘配額及重設倒數。
- **重設額度**：每個帳號的額度記錄，含到期時間與剩餘時間。
- **擷取用量**：一鍵擷取單一帳號或全部帳號的配額，並可設定自動擷取間隔。
- **帳號管理**：透過 Codex 登入新增帳號、封存不用的帳號，並安全切換目前帳號：Codex 會關閉、保留最新權杖、寫入所選工作階段，然後自動重新開啟。
- **選單列彈出視窗（macOS）**：選單列標題顯示目前帳號的 5 小時與每週配額；點按可查看精簡帳號清單、一步切換並開啟主畫面。
- **即時記錄**：固定高度的活動面板，支援一鍵複製和清除。
- **設定**：介面語言、選單列顯示、桌面通知和自動檢查更新，修改後立即儲存。
- **應用程式內更新**：簽章更新來源，支援手動檢查，並每 6 小時自動檢查。

## 語言

介面支援英文、簡體中文、繁體中文和日文。在 **設定 → 語言** 中選擇，或保留 **系統** 跟隨作業系統語言。選單列原生選單和主畫面使用同一設定，應用程式狀態訊息也會跟隨該設定。後端診斷記錄保持英文，方便在問題報告中排查。

## 使用方式

1. 啟動應用程式，點按 **新增帳號**，透過 Codex 登入。
2. 點按 **重新整理配額**（或等待自動擷取）取得目前帳號用量；點按 **擷取所有帳號** 取得全部備用帳號。
3. 將滑鼠停留在工具列按鈕上可查看說明；點按 **眼睛** 切換封存帳號，點按 **太陽/月亮** 切換明暗主題，點按 **齒輪** 開啟設定。
4. 點按底部的 **活動** 展開或收合記錄。切換帳號只需確認一次，關閉 Codex、切換憑證和重新開啟會作為一步完成。
5. 在 macOS 上，關閉視窗後 Codex Monitor 會繼續在選單列執行。使用選單列彈出視窗重新開啟主畫面，或從選單選擇 **結束**。如果在設定中關閉 **顯示選單列圖示**，關閉視窗將直接結束應用程式。

## 更新機制

- **雲端下載** 按鈕會檢查最新版本中的 `latest.json`。若有更新，會下載對應系統和架構的安裝檔、驗證簽章、安裝並重新啟動。
- 啟動時以及每 6 小時還會靜默檢查。結果顯示在狀態列，不會彈出視窗。
- 所有平台建置完成後，CI 會重新產生 `latest.json`，確保它始終指向同一版本的資源。因此已發布的版本不會手動移動或修改。

## 開發

需要 Node 22 和穩定版 Rust 工具鏈。

```bash
cd apps/desktop
npm install
npm run tauri dev
```

常用指令（除另有註明外，均在 `apps/desktop` 下執行）：

```bash
npm run build          # 前端型別檢查（tsc）+ Vite 建置
npm test               # 前端單元測試
npm run tauri dev      # 含熱更新的完整應用程式
npm run tauri build    # 本機發布包（不會提交）
cargo test             # Rust 後端測試（apps/desktop/src-tauri）
cargo clippy --all-targets -- -D warnings
```

專案結構：

```text
apps/desktop/
  src/                 # React 前端（元件、狀態、i18n、格式化）
  src-tauri/           # Rust 後端（API、儲存、監看、更新）
scripts/
  set-version.mjs      # CI 中依發布標籤同步版本
  finalize-release.mjs # 正規化發布資源並產生 latest.json
  release-notes.mjs    # 從 CHANGELOG.md 擷取發布說明
```

## 發布

推送 `v2*` 標籤。CI 會建立草稿版本，建置 macOS arm64、macOS x64、Windows x64 和 Linux x64，然後統一資源名稱、重新產生 `latest.json`、寫入 `CodexMonitor-<version>-SHA256SUMS.txt`，最後一次發布：

```bash
git tag v2.6.0
git push origin v2.6.0
```

發布規則：

- **只發布 `v2*` 標籤。** 此工作流程只對 `v2*` 執行；不再發布 v1 版本。
- 已發布的版本不會移動或覆寫。如需修正，請發布新的修補版本。
- 本機建置的安裝檔不會提交（見 `.gitignore`）；發布包只來自 CI。
- 發布說明從 `CHANGELOG.md` 產生，請在建立標籤前加入對應版本章節。
