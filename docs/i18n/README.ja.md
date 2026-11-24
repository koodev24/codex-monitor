# Codex Monitor

[English](../../README.md) | [简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md) | 日本語

[![GitHub release](https://img.shields.io/github/v/release/koodev24/codex-monitor)](https://github.com/koodev24/codex-monitor/releases/latest)
[![Release build](https://img.shields.io/github/actions/workflow/status/koodev24/codex-monitor/release-v2-tauri.yml?label=release%20build)](https://github.com/koodev24/codex-monitor/actions)
![macOS](https://img.shields.io/badge/macOS-supported-blue)
![Windows](https://img.shields.io/badge/Windows-supported-blue)
![Linux](https://img.shields.io/badge/Linux-supported-blue)

すべての ChatGPT アカウントの Codex API 使用量を確認できるネイティブデスクトップアプリです。5 時間・週間クォータ、リセットまでのカウントダウン、アカウントごとのリセットクレジット、自動取得、署名検証付きのアプリ内更新に対応します。

## ダウンロード

最新版は [Releases ページ](https://github.com/koodev24/codex-monitor/releases/latest) から取得してください。各プラットフォームのアセット名は統一されています。

| OS | アーキテクチャ | ファイル |
| --- | --- | --- |
| macOS | Apple Silicon | `CodexMonitor-<version>-macos-arm64.dmg` |
| macOS | Intel | `CodexMonitor-<version>-macos-x64.dmg` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64-setup.exe` |
| Windows | x64 | `CodexMonitor-<version>-windows-x64.msi` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.AppImage` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.deb` |
| Linux | x64 | `CodexMonitor-<version>-linux-x64.rpm` |

各リリースには次のファイルも含まれます。

- `latest.json` — アプリが読み込む更新マニフェストで、各プラットフォームの署名付き更新ファイルを一覧にします。更新エンドポイントが固定名を要求するため、ファイル名にバージョンを含めません。手動で編集・再公開しないでください。
- `CodexMonitor-<version>-SHA256SUMS.txt` — ダウンロード可能なファイルのチェックサムです。
- `CodexMonitor-<version>-macos-*.app.tar.gz` と `.sig` ファイル — 更新用ペイロードと署名です。macOS の通常のインストールには DMG を使用します。

チェックサムファイルでダウンロードを検証できます:

```bash
shasum -a 256 -c CodexMonitor-<version>-SHA256SUMS.txt
```

### macOS での初回起動

アプリは署名されていないため、macOS が初回起動をブロックします。アプリを右クリックして **開く** を選ぶか、検疫フラグを一度解除してください。

```bash
xattr -dr com.apple.quarantine /Applications/CodexMonitor.app
open /Applications/CodexMonitor.app
```

アプリは `/Applications` または `~/Applications` に置いてください。アプリ内更新はバンドルをその場で置き換えます。

### v1 Python アプリからの移行

v2.6.0 から、旧 Python アップデーターが使用していた `CodexMonitor-macOS.zip` ブリッジの配布を終了しました。v1 を利用中の場合は手動で移行してください。

1. 旧アプリを終了します。
2. 最新リリースから Mac に合った DMG（Apple Silicon または Intel）をダウンロードします。
3. DMG を開き、**CodexMonitor.app** を `/Applications` または `~/Applications` に移動します。
4. 新しいアプリを起動します。初回起動時に旧データディレクトリがあれば、保存済みのアカウントと使用履歴を読み込みます。旧ファイルは読み取りのみで、削除されません。
5. アカウントが見つからない場合は、**アカウントを追加** からサインインするか、保存済みの `auth.json` をインポートしてください。その後、旧 Python アプリを削除できます。

## 機能

- **複数アカウントのクォータ** — アカウントごとに 5 時間・週間の残りクォータとリセットまでのカウントダウンを表示します。
- **リセットクレジット** — アカウントごとの有効期限と残り時間を一覧表示します。
- **取得** — 1 アカウント単位または全アカウントのクォータをワンクリックで取得し、自動取得間隔も設定できます。
- **アカウント管理** — Codex ログインでアカウントを追加し、不要なアカウントをアーカイブできます。アクティブアカウントの切り替えは安全に行われ、Codex を閉じ、最新トークンを保持し、選択したセッションを書き込んでから自動的に再起動します。
- **メニューバーポップオーバー（macOS）** — メニューバーにアクティブアカウントの 5 時間・週間クォータを表示します。クリックするとコンパクトなアカウント一覧、ワンステップの切り替え、ダッシュボードを開けます。
- **ライブログ** — 固定高さのアクティビティパネルで、ワンクリックのコピーとクリアに対応します。
- **設定** — 表示言語、メニューバー表示、デスクトップ通知、自動更新チェックを変更でき、即座に保存されます。
- **アプリ内更新** — 署名付き更新フィードを使用し、手動チェックと 6 時間ごとの自動チェックを行います。

## 言語

インターフェースは英語、簡体字中国語、繁体字中国語、日本語に対応しています。**設定 → 言語** で選択するか、**システム** のままで OS の言語に追従させます。メニューバーのネイティブメニューとダッシュボードは同じ設定を使用し、アプリのステータスメッセージも追従します。バックエンドの診断ログは英語のままです。不具合報告で調査しやすくするためです。

## 使い方

1. アプリを起動し、**アカウントを追加** をクリックして Codex ログインを行います。
2. **クォータを更新**（または自動取得を待つ）でアクティブアカウントの使用量を取得します。**すべてのアカウントを取得** でバックアップアカウントも取得できます。
3. ツールバーのボタンにマウスを合わせると説明が表示されます。**目** でアーカイブ済みアカウントの表示を切り替え、**太陽/月** でライト/ダークテーマを切り替え、**歯車** で設定を開きます。
4. 下部の **アクティビティ** をクリックするとログを展開・折りたたみできます。アカウント切り替えは一度確認するだけで、Codex の終了、認証情報の切り替え、再起動を一連の操作として実行します。
5. macOS では、ウィンドウを閉じても Codex Monitor はメニューバーで動作を続けます。トレイのポップオーバーからダッシュボードを開くか、メニューの **終了** で終了します。設定で **メニューバー項目を表示** をオフにすると、ウィンドウを閉じた時点でアプリが終了します。

## 更新の仕組み

- **クラウドダウンロード** ボタンは最新リリースの `latest.json` を確認します。更新がある場合は OS とアーキテクチャに合ったファイルをダウンロードし、署名を検証してインストールし、再起動します。
- 起動時と 6 時間ごとにもバックグラウンドで確認します。結果はステータスバーに表示され、ポップアップは出ません。
- `latest.json` は全プラットフォームのビルド完了後に CI が再生成するため、常に同じリリースのアセットを指します。そのため公開済みリリースを手動で移動・編集しません。

## 開発

Node 22 と安定版 Rust ツールチェーンが必要です。

```bash
cd apps/desktop
npm install
npm run tauri dev
```

よく使うコマンド（特記がない限り `apps/desktop` で実行）：

```bash
npm run build          # フロントエンドの型チェック（tsc）+ Vite ビルド
npm test               # フロントエンドのユニットテスト
npm run tauri dev      # ホットリロード付きのアプリ
npm run tauri build    # ローカル用リリースバンドル（コミットしない）
cargo test             # Rust バックエンドのテスト（apps/desktop/src-tauri）
cargo clippy --all-targets -- -D warnings
```

プロジェクト構成：

```text
apps/desktop/
  src/                 # React フロントエンド（コンポーネント、ストア、i18n、書式）
  src-tauri/           # Rust バックエンド（API、ストレージ、監視、更新）
scripts/
  set-version.mjs      # CI でリリースタグからバージョンを同期
  finalize-release.mjs # アセット名を統一し latest.json を生成
  release-notes.mjs    # CHANGELOG.md からリリースノートを抽出
```

## リリース

`v2*` タグをプッシュします。CI はドラフトリリースを作成し、macOS arm64、macOS x64、Windows x64、Linux x64 をビルドした後、アセット名を統一し、`latest.json` を再生成し、`CodexMonitor-<version>-SHA256SUMS.txt` を書き込み、最後に一度だけ公開します。

```bash
git tag v2.6.0
git push origin v2.6.0
```

リリースのルール：

- **`v2*` タグのみを使用してください。** このワークフローは `v2*` でのみ実行されます。v1 リリースは生成されません。
- 公開済みリリースは移動・上書きしません。修正が必要な場合は新しいパッチバージョンをリリースしてください。
- ローカルビルドのバンドルはコミットしません（`.gitignore` を参照）。リリースは CI からのみ生成します。
- リリース本文は `CHANGELOG.md` から生成されるため、タグを作成する前に該当バージョンのセクションを追加してください。
