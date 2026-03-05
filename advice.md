# Objective
`src-tauri/src/lib.rs` の分析スレッドおよび状態管理のコアロジックを、Windows版で実装された最新のバグ修正や安全対策に合わせてアップデートしてください。ただし、Mac固有の処理（センサー、ファイアウォール回避、Finderでのフォルダ展開など）は絶対に削除・破壊しないでください。

## Context
Windows版のプロトタイプ開発において、HMMエンジンの時間加速（Time Dilation）や無入力時のドリフト、セッション開始時の打鍵ロスを防ぐための重要な修正が行われました。現在、Mac版（このリポジトリ）の `lib.rs` にはこれらの修正が反映されていません。コードベースを統合するための準備として、Mac版のロジックをWindows版の最新状態と同期させる必要があります。

## Requirements (Step-by-Step Instructions)

以下の5つの変更を `src-tauri/src/lib.rs` に適用してください。

### 1. セッション開始時（ResetSignal受信時）のキュー・ドレインの廃止
- **問題:** 現在のMac版では、分析スレッド内で `reset_signal_for_thread` を処理する際、`while rx.try_recv().is_ok() {}` でチャネルをドレイン（破棄）しています。これにより、セッション開始直後の有効な打鍵が失われるリスクがあります。
- **修正:** `reset_signal_for_thread` の処理ブロック内から `while rx.try_recv().is_ok() {}` を**削除**してください。`extractor.reset();` などのリセット処理や時間変数の初期化はそのまま残します。

### 2. 無入力期間（Timeout時）のHMM更新メソッドの変更
- **問題:** 無入力期間中（`RecvTimeoutError::Timeout`）に、通常の打鍵時と同じ `engine_for_thread.update(&sf, ime_open)` を呼んでいるため、EWMA（指数移動平均）がドリフトする問題があります。
- **修正:** タイムアウト時のHMM更新処理を `engine_for_thread.update_silence(&sf, ime_open);` に変更してください。（※ `CognitiveStateEngine` に既に実装されている想定です）

### 3. IMEモニタースレッドのフェイルセーフ（8秒タイムアウト）の導入
- **問題:** 現在のMac版のIMEモニターは、候補ウィンドウの状態をポーリングしているだけです。OSイベントの不具合で状態がスタックした場合、HMMが永久に停止します。
- **修正:** Windows版と同様に、IMEがアクティブな状態が **8秒以上継続** した場合、OSイベントの取りこぼし（stale）とみなして `IME_ACTIVE` を `false` に自動リセットし、`engine_for_monitor.set_paused(false);` を呼び出す安全装置（フェイルセーフ）のロジックを追加してください。（`Instant::now()` を用いて継続時間を計測します）

### 4. `start_session` コマンドの状態変更順序の修正
- **問題:** 現在のMac版の `start_session` 関数では、エンジンリセット→ResetSignal送信→SessionActive有効化、という順序になっています。
- **修正:** スレッド間の競合を防ぐため、順序を以下のように変更してください。
  1. `active.0.store(true, Ordering::Release);` (セッションを有効化)
  2. `engine.reset();` (エンジンリセット)
  3. `reset.0.store(true, Ordering::Release);` (分析スレッドへリセットシグナル送信)
  4. ログ記録 (変更なし)

### 5. Mac固有コードの厳格な維持（Do NOT Modify）
以下のMac固有の実装は変更せず、そのまま維持してください。
- `tauri::Builder::default().setup(...)` 内の `SensorManager` の初期化と開始。
- アプリ起動時のダミーTCPバインド（ファイアウォール回避策）。
- `quit_app` 関数内のセッションフォルダを開く処理（`std::process::Command::new("open")` を使用している点）。
- 権限確認を行う `get_hook_status` 関数の内容（`#[cfg(target_os = "macos")]` を含む処理）。

## Execution
ファイルを修正した後、`cargo check` を実行してコンパイルエラーが発生しないことを確認してください。