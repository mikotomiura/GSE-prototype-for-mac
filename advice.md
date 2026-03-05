# Objective
フロントエンドのコンポーネント（`Dashboard.tsx` と `Overlay.tsx`）のロジックを修正し、Windows版で実装済みの堅牢な時間計算と打鍵判定ロジックを統合してください。現在のPropsやUI構造（権限バナーなど）は絶対に維持してください。

## Context
現在のMac版フロントエンドには2つのマイナーな論理的欠陥があります。
1. `Dashboard` のセッションタイマーが `setInterval` で `prev + 1` されているため、長時間の実行や高負荷時にブラウザのタイマーが遅れ、現実の時間からドリフトしてしまいます。
2. `Overlay` の打鍵警告（Typing Warning）が、単純に `keyboardIdleMs < 3000` で判定されているため、Wallが発動した「直前の打鍵」に反応してしまい、Wall表示直後に必ず誤警告が出てしまう問題（False Positive）があります。

## Requirements (Step-by-Step Instructions)

### Step 1: `Dashboard.tsx` のタイマーロジック修正
- `src/components/Dashboard.tsx` を開き、`sessionSeconds` の更新ロジックを絶対時間ベースに変更してください。
- `useRef(Date.now())` を用いてセッション開始時刻を記録し、1秒ごとの `setInterval` の中では `Math.floor((Date.now() - sessionStartRef.current) / 1000)` をセットするように書き換えてください。

### Step 2: `Dashboard.tsx` のイベント名の統一
- `handleMonkModeToggle` 内の `emit` イベント名を `"monk-mode-change"` から `"monk-mode-changed"` に変更してください（バックエンド・他OSとの互換性のため）。

### Step 3: `Overlay.tsx` の打鍵警告ロジックの修正
- `src/components/Overlay.tsx` を開いてください。
- 単純な `keyboardIdleMs` の判定による警告を廃止し、以下のWindows版の堅牢なロジックを移植してください。
  1. `wallStartTimeRef` (Wall発動時刻) と `lastWarnedKeyAtRef` (最後に警告した打鍵時刻) の `useRef` を追加する。
  2. `isWallActive` が `true` になった瞬間に `wallStartTimeRef` に `Date.now()` をセットする。
  3. `isWallActive` 中は `setInterval(..., 300)` で `invoke<number>("get_last_keypress_timestamp")` を呼び出す。
  4. 取得した `lastKeyAt` が `wallStartTimeRef` より大きく（Wall発動**後**の打鍵であり）、かつ `lastWarnedKeyAtRef.current + 1000` より大きい（1秒のデバウンス）場合にのみ、警告バナーを2.5秒間表示するロジックを実装する。
- 既存の `phoneConnected` などのUI表示ロジックは一切変更しないでください。

## Execution
修正後、`npm run build` または `npm run dev` を実行し、TypeScriptのコンパイルエラーが出ないことを確認してください。