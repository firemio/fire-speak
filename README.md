# fire-speak 🔥

Genspark Speakly 代替の常駐型AI音声入力アプリ（Windows 11 / Tauri v2）。

**ホットキー → 話す → 文字起こし → AI整形 → アクティブなアプリに自動貼り付け。**

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

## 開発・起動

```bash
npm install
npm run tauri dev
```

配布用ビルド（NSISインストーラが `src-tauri/target/release/bundle/` に生成される）:

```bash
npm run tauri build
```

## 初回セットアップ

1. アプリ起動 → 設定画面「**セットアップ**」タブ
2. **Whisperサーバをインストール**（whisper.cpp 公式リリースを自動DL・展開）
3. **モデルをダウンロード**（推奨: `small`＝488MB で日本語実用、精度重視なら `large-v3-turbo`＝1.6GB）
4. 「**AI整形**」タブでプロバイダにAPIキーを設定
   - Claude: [Anthropic Console](https://console.anthropic.com/) でキー発行
   - Laguna S 2.1 無料枠: [OpenRouter](https://openrouter.ai/) でキー発行（`poolside/laguna-s-2.1:free`）
5. どこかテキスト入力欄にカーソルを置いて `Ctrl+Alt+Space` → 話す → もう一度押す → 貼り付き完了

APIキーなし・完全ローカルでも「そのまま」モード（整形なし）で動作します。

## アップデートの配布方法（開発者向け）

アップデート確認は GitHub Releases の最新リリースを見ます。新版を配る手順:

1. `tauri.conf.json` と `package.json` の `version` を上げてビルド: `npm run tauri build`
2. GitHubリポジトリ（既定: `firemio/fire-speak`。設定で変更可）にタグ `vX.Y.Z` でリリースを作成
3. `fire-speak_X.Y.Z_x64-setup.exe` をリリースにアップロード（`-setup.exe` で終わるアセットがダウンロードボタンの飛び先になる）

## 設定の保存先

`%APPDATA%\jp.firemio.fire-speak\`（settings.json / history.json / bin / models）

## トラブルシュート

- **貼り付かない**: 対象アプリが管理者権限で動いていると通常権限アプリからのキー送信はブロックされます。fire-speak も管理者で起動するか、`paste_mode: clipboard`（コピーのみ）にして手動 `Ctrl+V`
- **ローカル文字起こしが遅い**: モデルを `small` 以下にする、または設定でスレッド数を増やす
- **ホットキーが効かない**: 他アプリと衝突している可能性。「一般」タブで変更
