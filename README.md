# fire-speak 🔥

[![Latest release](https://img.shields.io/github/v/release/firemio/fire-speak?label=Download&color=ff6b35)](https://github.com/firemio/fire-speak/releases/latest)

Genspark Speakly 代替の常駐型AI音声入力アプリ（Windows 11 / Linux / Tauri v2）。

**ホットキー → 話す → 文字起こし → AI整形 → アクティブなアプリに自動貼り付け。**

## 📥 ダウンロード

**[最新版をダウンロード](https://github.com/firemio/fire-speak/releases/latest)**

| OS | Assets から選ぶファイル |
|---|---|
| Windows 11 (x64) | `fire-speak_X.Y.Z_x64-setup.exe`（または `.msi`） |
| Linux Mint / Ubuntu / Debian (x86_64) | `fire-speak_X.Y.Z_amd64.deb` |
| Linux Mint / Ubuntu / Debian (ARM64) | `fire-speak_X.Y.Z_arm64.deb` |
| Fedora / RHEL 系 | `fire-speak-X.Y.Z-1.x86_64.rpm` / `...aarch64.rpm` |
| その他の Linux | `fire-speak_X.Y.Z_amd64.AppImage` / `..._aarch64.AppImage` |

Linux は[Linux でのインストール](#linux-でのインストール)を必ず読んでください（**Wayland では動きません**）。

ソースからビルドする場合は[開発・起動](#開発起動)へ。

## 特徴

- **グローバルホットキー**（既定: `Ctrl+Alt+Space`）でどのアプリ上でも起動。トグルで録音開始/確定
- **文字起こしエンジン切替**
  - ローカル: whisper.cpp（完全無料・オフライン・プライバシー安全。アプリ内からサーバ/モデルをワンクリック導入）
  - クラウド: OpenAI互換 `audio/transcriptions` API（OpenAI / Groq など）
- **AI整形プロバイダ切替**
  - Anthropic API（Claude Haiku 等）
  - OpenAI互換 chat/completions（OpenRouter の Laguna S 2.1 無料枠、Groq、Ollama ローカル等）
- **モード**: 整形 / そのまま / 英語に翻訳 / 日本語に翻訳 / ターミナルコマンド / ビジネス文体 + カスタム追加可。トレイメニューからワンクリック切替
- **録音HUD**: フォーカスを奪わない波形オーバーレイ
- **履歴**: 直近の入力を保存・コピー可能
- LLM障害時は生テキストで続行（貼り付けが止まらない）
- **多言語UI（12言語）**: 日本語 / English / 简体中文 / 繁體中文 / 한국어 / Español / Français / Deutsch / Português (Brasil) / Русский / Tiếng Việt / Bahasa Indonesia。初回はOS言語を自動検出、「一般」タブで即時切替（トレイ・HUD・エラー文言まで追随）
- **アップデート確認**: 起動時の自動チェック+手動チェック。GitHub Releases の新版を検知するとホームに通知し、ダウンロードページを開ける（「アップデート」タブで自動チェックON/OFF・参照リポジトリ変更可）

## Linux でのインストール

### ⚠️ 先に読むこと — セッションは X11 にしてください

fire-speak のグローバルホットキーと自動貼り付け（Ctrl+V の合成入力）は **X11 が前提**です。

- **Wayland セッションでは動作しません。** Wayland はセキュリティ設計上、アプリが他アプリのキー入力を横取りすること（グローバルホットキー）も、他アプリへキーを送り込むこと（自動貼り付け）も禁止しています。fire-speak は起動しますが、ホットキーが反応しません。
- ログイン画面の歯車アイコン等から **X11 / Xorg セッション**を選んでください。
- **Linux Mint（Cinnamon）は既定が X11** なので、そのまま使えます。Ubuntu 22.04 以降と Fedora は既定が Wayland なので、切り替えが必要です。

現在のセッション種別は次で確認できます:

```bash
echo $XDG_SESSION_TYPE   # x11 と出れば OK
```

### Linux Mint / Ubuntu / Debian（.deb）

```bash
# ダウンロードしたディレクトリで
sudo apt install ./fire-speak_0.3.0_amd64.deb
```

`apt install ./ファイル名` にするのが重要です（`dpkg -i` だと依存パッケージを自動で入れてくれません）。ARM64 機（Raspberry Pi 等）は `_arm64.deb` を使います。

アンインストール:

```bash
sudo apt remove fire-speak
```

### Fedora / RHEL 系（.rpm）

```bash
sudo dnf install ./fire-speak-0.3.0-1.x86_64.rpm
```

アンインストール:

```bash
sudo dnf remove fire-speak
```

### その他のディストリ（AppImage）

```bash
chmod +x fire-speak_0.3.0_amd64.AppImage
./fire-speak_0.3.0_amd64.AppImage
```

AppImage は依存ライブラリを同梱しますが、**webkit2gtk 4.1 は同梱されません**。動かない場合は下記のランタイム依存を手動で入れてください。

### ランタイム依存パッケージ

`.deb` / `.rpm` は依存関係として自動で入ります。AppImage や手動ビルドの場合のみ必要です。

| 用途 | Debian / Ubuntu / Mint | Fedora |
|---|---|---|
| WebView（UI本体） | `libwebkit2gtk-4.1-0` | `webkit2gtk4.1` |
| トレイアイコン | `libayatana-appindicator3-1` | `libayatana-appindicator-gtk3` |
| マイク録音 | `libasound2t64`（22.04 では `libasound2`） | `alsa-lib` |
| ホットキー / 自動貼り付け | `libxdo3`, `libx11-6`, `libxtst6` | `xdotool`, `libX11`, `libXtst` |
| フォルダ・URL を開く | `xdg-utils` | `xdg-utils` |

### 既知の制限（Linux）

正直に書いておきます。Windows 版と同じようには動きません。

- **ホットキーを押している間、他のキー入力は前面アプリに届きません。** Linux 版は X11 の排他グラブでホットキーを横取りするため、Windows 版と同じくホットキー自体は前面アプリに漏れません（右 Alt を押してもアプリのメニューは開きません）。ただし X11 の仕様上、グラブが有効な間＝**ホットキーを押している間**は他のキー入力も fire-speak 側に吸われます。話しながら同時にタイプすることは想定しにくいので許容していますが、Windows 版との唯一の動作差です。
- **ホットキーを他のアプリに先に取られていると設定できません。** クリップボードマネージャや IME、ウィンドウマネージャが同じキーを掴んでいる場合、排他グラブに失敗して「一般」タブでの設定がエラーになります。その場合は別のキーか組み合わせを選んでください。
- **Wayland セッションではグローバルホットキーを取得できません。** 上記のとおり、これは fire-speak 側では回避できません。X11 セッションを使ってください。
- **管理者権限のアプリへの貼り付け**という Windows 特有の制限は Linux にはありませんが、代わりに XTEST 入力を無視するアプリ（一部のゲーム・リモートデスクトップ）には貼り付けできません。その場合は「一般」タブの `paste_mode` を `clipboard`（コピーのみ）にして手動で `Ctrl+V` してください。
- ディストリの webkit2gtk が古すぎる／新しすぎる環境向けの Flatpak は、まだ配布していません（`packaging/` に準備中の manifest があります）。

## 開発・起動

```bash
npm install
npm run tauri dev
```

Linux でビルドする場合は、先にビルド依存を入れてください:

```bash
# Debian / Ubuntu / Linux Mint
sudo apt install build-essential curl wget file \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev \
  patchelf xdg-utils libasound2-dev libxdo-dev \
  libx11-dev libxi-dev libxtst-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev

# Fedora
sudo dnf group install c-development
sudo dnf install webkit2gtk4.1-devel libayatana-appindicator-gtk3-devel \
  librsvg2-devel openssl-devel curl wget file patchelf xdg-utils \
  alsa-lib-devel libxdo-devel \
  libX11-devel libXi-devel libXtst-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel
```

配布用ビルド（インストーラが `src-tauri/target/release/bundle/` に生成される）:

```bash
npm run tauri build
```

生成物はOSごとに自動で切り替わります — Windows なら NSIS (`-setup.exe`) と MSI、Linux なら deb / rpm / AppImage。

## 初回セットアップ

1. アプリ起動 → 設定画面「**セットアップ**」タブ
2. **Whisperサーバをインストール**（whisper.cpp 公式リリースを自動DL・展開）
3. **モデルをダウンロード**（推奨: `small`＝488MB で日本語実用、精度重視なら `large-v3-turbo`＝1.6GB）
4. 「**AI整形**」タブでプロバイダにAPIキーを設定
   - Claude: [Anthropic Console](https://console.anthropic.com/) でキー発行
   - Laguna S 2.1 無料枠: [OpenRouter](https://openrouter.ai/) でキー発行（`poolside/laguna-s-2.1:free`）
5. どこかテキスト入力欄にカーソルを置いて `Ctrl+Alt+Space` → 話す → もう一度押す → 貼り付き完了

APIキーなし・完全ローカルでも「そのまま」モード（整形なし）で動作します。

## リリースの作り方（開発者向け）

ビルドと配布は GitHub Actions（`.github/workflows/release.yml`）が行います。手元でビルドしてアップロードする必要はありません。

1. `src-tauri/tauri.conf.json` と `package.json` の `version` を上げてコミット
2. タグを打って push:
   ```bash
   git tag v0.3.1
   git push origin v0.3.1
   ```
3. ワークフローが Windows x86_64 / Linux x86_64 / Linux aarch64 / Windows arm64 の4ジョブを並列で走らせ、全成果物を**下書き（draft）リリース**に添付します
4. 全ジョブが終わったら GitHub の Releases 画面で **Publish release** を押す

下書きにしているのは意図的です — アプリ内のアップデート確認は `releases/latest` を見ており、下書きは無視されるため、アセットが揃う前のリリースをユーザーに見せずに済みます。

補足:

- アプリ内のアップデート確認は、**実行中の環境と同じアーキテクチャ**のアセット（名前に `x64`/`amd64`/`x86_64` または `arm64`/`aarch64` を含むもの）だけをダウンロード先にします。該当が無い場合はリリースページを開きます
- タグを打たずに手動で走らせたい場合は Actions 画面から `release` ワークフローを `workflow_dispatch` で起動し、`tag` 入力にタグ名を指定します
- push / PR ごとの `ci.yml` は Windows と Linux x86_64 でコンパイルだけを確認します（パッケージングもリリースもしません）

## 設定の保存先

- Windows: `%APPDATA%\jp.firemio.fire-speak\`
- Linux: `~/.config/jp.firemio.fire-speak/`（settings.json / history.json）と `~/.local/share/jp.firemio.fire-speak/`（bin / models）

## トラブルシュート

- **貼り付かない（Windows）**: 対象アプリが管理者権限で動いていると通常権限アプリからのキー送信はブロックされます。fire-speak も管理者で起動するか、`paste_mode: clipboard`（コピーのみ）にして手動 `Ctrl+V`
- **貼り付かない・ホットキーが効かない（Linux）**: まず `echo $XDG_SESSION_TYPE` が `x11` か確認してください。`wayland` なら X11 セッションでログインし直す必要があります（「Linux でのインストール」の「既知の制限」参照）
- **ローカル文字起こしが遅い**: モデルを `small` 以下にする、または設定でスレッド数を増やす
- **ホットキーが効かない**: 他アプリと衝突している可能性。「一般」タブで変更。Linux では押したキーが前面アプリにも届くため、他アプリが使っていない組み合わせを選んでください
