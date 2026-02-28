# GSE — Generative Struggle Engine for macOS

> **macOS 上でキーストローク動的特徴からリアルタイムに認知状態を推定するシステム**
> クラウド依存なし・ユーザー介入なしで、手動チューニングされた隠れマルコフモデル（HMM）を用いて
> 書き手の精神状態を **Flow / Incubation / Stuck** に分類します。

[🇺🇸 English README is here](README.md)

---

## 目次

1. [研究動機](#研究動機)
2. [認知状態モデル](#認知状態モデル)
3. [システムアーキテクチャ](#システムアーキテクチャ)
4. [フォルダ構成](#フォルダ構成)
5. [特徴量抽出（F1–F6）](#特徴量抽出f1f6)
6. [HMM エンジン](#hmm-エンジン)
7. [ヒステリシスと安定性修正（v2.1）](#ヒステリシスと安定性修正v21)
8. [IME 検出とコンテキスト対応ベースライン](#ime-検出とコンテキスト対応ベースライン)
9. [macOS における IME モード検出：TIS + JIS キーアーキテクチャ](#macos-における-ime-モード検出tis--jis-キーアーキテクチャ)
10. [1Hz タイマー駆動推論と合成摩擦（v2.4）](#1hz-タイマー駆動推論と合成摩擦v24)
11. [介入 UI：Nudge & Wall（v2.5）](#介入-uinudge--wallv25)
12. [ログと分析](#ログと分析)
13. [ビルド手順](#ビルド手順)
14. [学術的参考文献](#学術的参考文献)

---

## 研究動機

ライター、プログラマー、知識労働者は **Flow（滑らかな高出力）**・**Incubation（意識的な停止と潜在的処理）**・**Stuck（認知的行き詰まり、非生産的ループ）** の三状態を交互に経験します。これらの状態をリアルタイムで把握できれば、アンビエント音楽・ナッジ・UI ディミングなど、タスクを中断しないまま認知的足場（metacognitive scaffolding）を提供するアダプティブツールが実現できます。

既存のアプローチはウェアラブルデバイス・カメラ・明示的自己報告を必要とします。本プロトタイプは OS から既に取得可能な**キーストロークタイミング情報のみ**を使用するため、追加ハードウェアなしで任意の macOS デバイスに展開可能です。

---

## 認知状態モデル

三状態は認知科学の確立された文献に基づいて定義されています。

| 状態 | 定義 | 行動シグネチャ |
| --- | --- | --- |
| **Flow** | 内発的に動機付けられた、努力なき課題への没入（Csikszentmihalyi, 1990） | 短いキー間隔、低修正率、長い連続バースト |
| **Incubation** | 潜在的な問題再構成を可能にする意図的停止（Sio & Ormerod, 2009） | **高い $P(\text{Burst} \mid \text{Pause})$**: 2 秒以上の沈黙 → 高速バーストの出現 |
| **Stuck** | 行き詰まりから抜け出せない固執状態（Ohlsson, 1992） | **高い $P(\text{Pause} \mid \text{Delete})$**: 削除→停止の固執ループ、文字純増ほぼゼロ |

---

## システムアーキテクチャ

```
┌─────────────────────────────────────────────────────────────────────┐
│                    macOS（Apple Silicon / Intel）                   │
│                                                                      │
│  ┌─────────────┐   CGEventTap       ┌──────────────────────────┐   │
│  │ 任意のアプリ │ ─────────────────── │  フックスレッド（Rust）  │   │
│  │ （フォアグラ │  （入力監視権限）   │  CGEventTapCreate        │   │
│  │  ウンド）   │                     │  CFRunLoop（専用スレッド）│   │
│  └─────────────┘                     └──────────┬───────────────┘   │
│                                                 │ crossbeam channel  │
│                                      ┌──────────▼───────────────┐   │
│                                      │  分析スレッド（Rust）    │   │
│                                      │  ── 1 Hz タイマーゲート ─│   │
│                                      │  FeatureExtractor        │   │
│                                      │    F1 フライトタイム中央値│   │
│                                      │    F2 フライトタイム分散 │   │
│                                      │    F3 修正率             │   │
│                                      │    F4 バースト長         │   │
│                                      │    F5 ポーズ回数         │   │
│                                      │    F6 削除後停止率       │   │
│                                      │                          │   │
│                                      │  CognitiveStateEngine    │   │
│                                      │    潜在軸（X, Y）        │   │
│                                      │    EWMA 平滑化           │   │
│                                      │    HMM 前向きアルゴリズム│   │
│                                      │    ヒステリシス EMA 層   │   │
│                                      └──────────┬───────────────┘   │
│                                                 │ Tauri IPC          │
│                                      ┌──────────▼───────────────┐   │
│                                      │  React/TS フロントエンド │   │
│                                      │  ダッシュボード+オーバーレイ│  │
│                                      │  Lv1 Nudge（赤い霧）    │   │
│                                      │  Lv2 Wall（全画面遮断）  │   │
│                                      └──────────────────────────┘   │
│                                                 │                    │
│                                      ┌──────────▼───────────────┐   │
│                                      │  SessionLogger（Rust）   │   │
│                                      │  NDJSON → Documents/     │   │
│                                      │  GSE-sessions/           │   │
│                                      └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### スレッドモデル

```
メインスレッド（Tauri イベントループ + TIS 用 GCD メインキュー）
    │
    ├─ フックスレッド          ← CGEventTap コールバック + CFRunLoop（専用スレッド）
    │       │ crossbeam::channel (bounded 64, ノンブロッキング送信)
    │       │ bounded(1) ウェイクチャンネル → IME ポーリングスレッド
    ├─ 分析スレッド            ← recv_timeout(1 s) → 1 Hz タイマーゲート; HMM 更新 ≤ 1回/秒
    │       │ Arc<Mutex<CognitiveStateEngine>> (Tauri managed state)
    │       │ 合成摩擦: 沈黙 ≥ 20 s → F6/F3 を Stuck 方向へ漸増
    ├─ IME モニタースレッド    ← is_candidate_window_open() 100ms ポーリング（スタブ：常に false）
    ├─ IME ポーリングスレッド  ← recv_timeout(100ms) ウェイク; TIS を dispatch_sync_f でメインキューへ
    │
    ├─ ロガースレッド          ← bounded channel(512) → NDJSON ファイル (BufWriter)
    └─ センサースレッド        ← スタブ（macOS では加速度センサー未実装）
```

---

## フォルダ構成

```
GSE-prototype/
├── analysis/
│   ├── behavioral_gt.py         # セッション後行動ルールベース GT ラベリング
│   └── hmm_sensitivity.py       # HMM パラメータ感度分析
│
├── src/                         # React / TypeScript フロントエンド
│   ├── components/
│   │   ├── Dashboard.tsx        # 状態確率バー + セッション情報
│   │   └── Overlay.tsx          # Nudge（赤い霧）+ Wall（全画面遮断）
│   ├── App.tsx                  # 介入ステートマシン（Lv1 → Lv2 エスカレーション）
│   └── main.tsx
│
├── src-tauri/                   # Rust / Tauri 2.0 バックエンド
│   ├── capabilities/
│   │   └── default.json         # Tauri 2.0 ケイパビリティ宣言
│   ├── src/
│   │   ├── analysis/
│   │   │   ├── engine.rs        # HMM エンジン + ヒステリシス層（display_probs EMA）
│   │   │   ├── features.rs      # F1–F6 特徴量抽出 + 沈黙合成
│   │   │   └── mod.rs
│   │   ├── input/
│   │   │   ├── hook.rs          # 共有アトミック（IME_OPEN, EVENT_SENDER など）
│   │   │   ├── hook_macos.rs    # CGEventTap 実装（macOS キーボードフック）
│   │   │   ├── ime.rs           # ImeMonitor スタブ + TIS ポーリングディスパッチャ
│   │   │   ├── ime_macos.rs     # TIS Carbon FFI（dispatch_sync_f でメインキューへ）
│   │   │   └── mod.rs
│   │   ├── lib.rs               # Tauri セットアップ、スレッド管理、IPC コマンド
│   │   ├── logger.rs            # 非同期 NDJSON セッションロガー
│   │   ├── main.rs
│   │   ├── sensors.rs           # センサーディスパッチャ（macOS スタブ）
│   │   └── sensors_macos.rs     # 加速度センサースタブ（未実装）
│   ├── entitlements.mac.plist   # App Sandbox 無効（研究プロトタイプ）
│   ├── Info.mac.plist           # NSInputMonitoringUsageDescription
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── index.html
├── package.json
├── tsconfig.json
└── vite.config.ts
```

---

## 特徴量抽出（F1–F6）

全特徴量は生のキーストロークイベントの **30 秒スライディングウィンドウ**で計算され、キー押下ごとに更新されます。無入力中（沈黙）は `make_silence_observation()` が 1 秒ごとに合成観測値を生成し、HMM の継続更新を行います。

| 特徴量 | 記号 | 定義 | 認知的意味 |
| --- | --- | --- | --- |
| フライトタイム中央値 | **F1** | キー離し→次キー押下間隔（ms）の直近 5 サンプル中央値 | タイピング速度 — 低いほど Flow |
| フライトタイム分散 | **F2** | ウィンドウ内フライトタイムの分散 | リズムの一貫性 |
| 修正率 | **F3** | (Backspace + Delete 押下数) / 全押下数 | エラー頻度 — 高いほど Stuck |
| バースト長 | **F4** | 連続キー入力チャンク（間隔 < 200 ms）の平均文字数 | 出力流暢性 — 高いほど Flow |
| ポーズ回数 | **F5** | ウィンドウ内でキー間隔 ≥ 2 000 ms の回数 | 熟考頻度 |
| 削除後停止率 | **F6** | Backspace/Delete 直後 ≥ 2 s 間隔が生じた割合 | 修正後フリーズ — 高いほど Stuck |

### 正規化関数 φ(x, β)

各生特徴量は片側線形正規化によって [0, 1] へマッピングされます。

```
φ(x, β) = clamp( (x − β) / (κ · β), 0.0, 1.0 )     κ = 2.0
```

β は集団中央値の推定値（固定参照値）。β 未満で 0.0、3β で 1.0 となります。

---

## HMM エンジン

### 意味的潜在軸：Cognitive Friction × Productive Silence

6 つの正規化特徴量を 2 つの解釈可能な意味的潜在軸に射影したうえで離散化します。

```text
X（Cognitive Friction）   = 0.30·φ(F3) + 0.25·φ(F6) + 0.25·φ(F1) + 0.20·φ(F5)
Y（Productive Silence）   = 0.40·φ(F4) + 0.35·(1 − φ(F1)) + 0.25·(1 − φ(F5))
```

両軸は指数加重移動平均（α = 0.30）で平滑化されます。

```
ewma_t = 0.30 · raw_t + 0.70 · ewma_{t−1}
```

### 観測ビン

(X, Y) ∈ [0,1]² を 5×5 グリッド（25 ビン）+ 1 ペナルティビン（obs=25；Backspace 5 連続以上）に離散化します。

```
認知摩擦 X →    0(低)   1      2      3      4(高)
生産的沈黙 Y ↓
4(高)      [Flow]  [Flow]  [   ]  [   ]  [    ]
3          [Flow]  [Flow]  [   ]  [   ]  [    ]
2          [    ]  [    ]  [ ? ]  [Stk]  [Stk ]
1          [Inc ]  [Inc ]  [Inc]  [Stk]  [Stk ]
0(低)      [Inc ]  [Inc ]  [ ? ]  [Stk]  [Stk ]
```

### HMM 前向きアルゴリズム（1 ステップ）

各更新で信念ベクトル **π** = [p_Flow, p_Inc, p_Stuck] を 1 ステップ伝播させます。

```
π'_j = ( Σ_i  π_i · A[i,j] ) · ( B[j, obs] + ε )    j ∈ {0, 1, 2}
π'   ← π' / Σ_j π'_j                                  （正規化）
```

- **A** = 3×3 遷移行列（行 = 遷移元、列 = 遷移先）
- **B** = 3×26 放射行列（状態 × 観測ビン）
- **ε** = 0.04（放射フロア）

### 遷移行列 A

| 遷移元 \ 先 | Flow | Incubation | Stuck |
| --- | --- | --- | --- |
| **Flow** | 0.80 | 0.13 | 0.07 |
| **Incubation** | 0.12 | 0.80 | 0.08 |
| **Stuck** | 0.06 | 0.18 | 0.76 |

---

## ヒステリシスと安定性修正（v2.1）

### 修正 ①：Cold-Start ヒステリシス

**問題：** 30 秒ウィンドウが大量 Backspace 区間を過ぎると、`p_stuck = 0.994 → p_flow = 0.48` が 1 ミリ秒で発生。

**修正：** 生 HMM 信念と並行して補助確率ベクトル `display_probs` を維持し、遅い EMA で追跡します。

```
display_t = α · raw_t + (1 − α) · display_{t−1}

α = 0.25  （通常更新 → 時定数 τ ≈ 4 秒）
α = 0.50  （Backspace ペナルティビン → 素早い Stuck 収束）
```

### 修正 ②：確率の離散クラスタリング

**修正：** 放射フロアを 0.01 → **0.04** に引き上げ。各状態の最大到達確率は 0.88〜0.90 程度に低下し、競合状態にも意味のある確率質量が残ります。

### 修正 ③：Inc→Stuck 沈黙遷移

**修正：** `make_silence_observation()` に沈黙時間に応じて線形増加する合成摩擦値を追加。

```rust
// F6：20 秒超で開始 → 80 秒で 0.50 に到達
F6_synthetic = clamp((silence_secs − 20) / 60,  0.0, 0.50)

// F3：30 秒超で開始 → 130 秒で 0.40 に到達
F3_synthetic = clamp((silence_secs − 30) / 100, 0.0, 0.40)
```

| 沈黙時間 | F3_合成 | F6_合成 | X（摩擦） | x_bin | 領域 |
| --- | --- | --- | --- | --- | --- |
| 20 秒 | 0.00 | 0.00 | ≈ 0.20 | 1 | Incubation |
| 30 秒 | 0.00 | 0.17 | ≈ 0.30 | 1 | Incubation |
| 40 秒 | 0.10 | 0.33 | ≈ 0.52 | 2 | 境界域 |
| 50 秒 | 0.20 | 0.50 | ≈ 0.75 | 3 | **Stuck** |

---

## IME 検出とコンテキスト対応ベースライン

日本語 IME 入力は特徴量分析に二つの課題をもたらします。

1. **候補選択フェーズ**（スペース → 漢字候補リスト表示中）：ナビゲーションキーが特徴量ベクトルを汚染する
2. **入力モードコンテキスト**（あモード vs Aモード）：フライトタイムの基準値が大きく異なる

### 候補ウィンドウ検出

macOS では `ImeMonitor::is_candidate_window_open()` は `false` を返すスタブです。候補選択中も HMM は動作し続けます（既知の制限）。

### IME モード検出（macOS）

macOS では二つの補完的なメカニズムで IME モードを検出します。

| メカニズム | 説明 | レイテンシ |
| --- | --- | --- |
| **JIS キー検出** | `kVK_JIS_Eisu`（0x66）/ `kVK_JIS_Kana`（0x68）を CGEventTap コールバックで検出 | < 5 ms |
| **TIS ポーリング** | `TISCopyCurrentKeyboardInputSource()` を 100ms ごとにポーリング（打鍵でウェイクアップ） | ≤ 100 ms |

**スレッド安全性：** TIS Carbon API は GCD メインキューで実行する必要があります。`is_japanese_ime_open()` は `dispatch_sync_f(&_dispatch_main_q, ...)` を通じてメインキューへマーシャリングし、`dispatch_assert_queue(main_queue)` クラッシュを回避します。

入力ソース ID に `"inputmethod.Japanese"` が含まれるかを検査します（Apple IME・Google 日本語入力など広範に対応）。

**ANSI/US キーボード：** JIS HID コードは届かないため、TIS ポーリング（100ms）が唯一の検出経路となります。両キーボードとも正しく動作します。

### コンテキスト別 β（デュアルベースライン）

| コンテキスト | `IME_OPEN` | β_F1（ms） | β_F3 | β_F4 | β_F5 | β_F6 |
| --- | --- | --- | --- | --- | --- | --- |
| **β_coding**（Aモード） | `false` | 150 | 0.06 | 5.0 | 2.0 | 0.08 |
| **β_writing**（あ/カモード） | `true` | 220 | 0.08 | 2.0 | 4.0 | 0.12 |

---

## macOS における IME モード検出：TIS + JIS キーアーキテクチャ

### アンチフラッピング設計

`LogEntry::ImeState` はキーボードフック・分析スレッドからは一切 emit されず、**IME ポーリングスレッドからのみ** emit されます。

```
CGEventTap コールバック（フックスレッド）
    │  atomic store: IME_OPEN = new_state  （JIS キーのみ）
    │  atomic store: IME_STATE_DIRTY = true
    └─ bounded(1) ウェイクチャンネルに try_send(())

IME ポーリングスレッド
    │  recv_timeout(100ms)   ← キープレス信号から 1ms 以内に起床
    │  sleep(5ms)            ← OS の IME 状態反映を待機
    │  dispatch_sync_f → メインキュー: TISCopyCurrentKeyboardInputSource()
    │  last_state（Option<bool>）と比較
    └─ LogEntry::ImeState { timestamp: SystemTime::now(), on } を emit
```

### キーアトミック（`hook.rs`）

| アトミック | 書き込み元 | 読み取り元 | 用途 |
| --- | --- | --- | --- |
| `IME_OPEN` | フック（JIS キー）、ポーリングスレッド | 分析スレッド | 現在の IME モード |
| `IME_STATE_DIRTY` | フック（JIS キー） | ポーリングスレッド（ドレイン） | ハウスキーピング |
| `IME_ACTIVE` | — | ImeMonitor（スタブ） | macOS では常に false |
| `HOOK_ACTIVE` | `hook_macos::start()` | `get_hook_status` コマンド | 権限バナー表示トリガー |

### `Option<bool>` 初期状態

ポーリングスレッドは `last_state: Option<bool> = None` で追跡し、初期状態が `false` であっても `true` であっても**初回検出は必ず emit** されます。

---

## 1Hz タイマー駆動推論と合成摩擦（v2.4）

分析スレッドは `recv_timeout(1000 ms)` により **1 Hz タイマーゲート**を実装します。

- **1 秒以内にキーストローク到着：** 特徴量を蓄積。前回の HMM ステップから 1 秒以上経過した場合のみ `engine.update()` を呼び出す。
- **タイムアウト：** `make_silence_observation()` が合成沈黙観測値を生成して HMM に入力。

これにより **1 秒あたり正確に 1 回の HMM 前向きステップ**が保証され、EMA 時定数 τ = 1/α ≈ **4 秒**がタイピング速度に依存せず数学的に正確に機能します。

---

## 介入 UI：Nudge & Wall（v2.5）

| レベル | 名称 | トリガー | 視覚効果 | ユーザー操作 |
| --- | --- | --- | --- | --- |
| **Lv1** | Nudge | p_stuck > 0.60 | 画面端の赤いビネット（霧） | クリックスルー（入力に対して透過） |
| **Lv2** | Wall | p_stuck > 0.70 が 30 秒持続 | 全画面遮断オーバーレイ + メッセージ | 解除まで全入力をブロック |

Wall の解除は**加速度センサー**イベント（`"sensor-accelerometer"` / `"move"` ペイロード）を使用します。macOS では加速度センサーが未実装のため、Wall はクリックスルーを手動で解除する必要があります（研究プロトタイプの制限）。

---

## ログと分析

セッションごとにタイムスタンプ付き NDJSON ファイルが生成されます。

```
~/Documents/GSE-sessions/gse_YYYYMMDD_HHMMSS.ndjson
```

レコードタイプ：

```jsonc
// セッションメタデータ
{"type":"meta","session_start":1740000000000}

// 生キーストロークイベント
{"type":"key","t":1740000001234,"vk":65,"press":true}

// 特徴量スナップショット + HMM 状態確率
{"type":"feat","t":1740000001235,
 "f1":145.20,"f2":312.00,"f3":0.0800,"f4":6.50,"f5":1.0,"f6":0.0000,
 "p_flow":0.7123,"p_inc":0.2100,"p_stuck":0.0777}

// IME モード切替
{"type":"ime_state","t":1740000001234,"on":true}

{"type":"meta","session_end":1740000060000}
```

### セッション後グラウンドトゥルースラベリング

```bash
python analysis/behavioral_gt.py ~/Documents/GSE-sessions/gse_YYYYMMDD_HHMMSS.ndjson
```

| ラベル | 行動ルール |
| --- | --- |
| **FLOW** | median(FT) < 200 ms かつ correction_rate < 0.15 かつ STUCK/INC でない |
| **INCUBATION** | Pause(≥2 s) → Burst(≥5 文字 FT<200 ms) → diff_chars ≥ 3（30 秒以内） |
| **STUCK** | 「Burst(≤3 文字) → Delete(≥1) → Pause(≥2 s)」のループ ≥ 3 回（60 秒内）かつ diff_chars ≤ 0 |
| **UNKNOWN** | 条件を満たさない、または複数ラベルが競合 |

---

## ビルド手順

### 前提条件

| ツール | バージョン |
| --- | --- |
| Rust | 1.77+（`rustup update stable`） |
| Node.js | 20+ |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |
| Xcode Command Line Tools | `xcode-select --install` |

### macOS 権限：入力監視（Input Monitoring）

GSE は CGEventTap を使用してキーストロークタイミングを取得します。初回起動時：

1. macOS が確認ダイアログを表示：*「GSEがキーボードからの入力を監視しようとしています」*
2. **システム設定 → プライバシーとセキュリティ → 入力監視** を開く
3. **GSE** の横のトグルを有効にする
4. アプリを再起動する

権限がない場合、`HOOK_ACTIVE` は `false` となり、ダッシュボードに黄色の警告バナーが表示されます。

### 開発実行

```bash
cd GSE-prototype
npm install
npm run tauri dev
```

### リリースビルド

```bash
npm run tauri build
# アプリバンドル: src-tauri/target/release/bundle/macos/
```

### コンパイル確認（UI なし）

```bash
~/.cargo/bin/cargo build --target aarch64-apple-darwin
```

---

## macOS 既知の制限

| 機能 | 状態 | 影響 |
| --- | --- | --- |
| IME コンポジション検出（`IME_ACTIVE`） | false 固定（スタブ） | 候補ウィンドウ中も HMM が動作 |
| 加速度センサー Wall 解除 | 未実装 | Wall は手動解除が必要 |
| JIS IME キー（ANSI キーボード） | TIS ポーリング（100ms）のみ | レイテンシの差は体感上無視可能 |
| 初回 Input Monitoring 権限 | 付与後にアプリ再起動が必要 | 初回セットアップのみ |

---

## 学術的参考文献

1. **Csikszentmihalyi, M.**（1990）. *Flow: The Psychology of Optimal Experience*. Harper & Row.
   — Flow 認知状態の定義とその行動的相関の基盤。

2. **Csikszentmihalyi, M.**（1996）. *Creativity: Flow and the Psychology of Discovery and Invention*. HarperCollins.
   — 創造的・生成的ライティング課題へのフロー理論の拡張。

3. **Sio, U. N., & Ormerod, T. C.**（2009）. Does incubation enhance problem solving? *Psychological Bulletin, 135*(1), 94–120.
   — Incubation 状態の自己遷移確率（0.80）と Pause→Burst 行動シグネチャの実証的根拠。

4. **Ohlsson, S.**（1992）. Information-processing explanations of insight and related phenomena. In *Advances in the Psychology of Thinking*（pp. 1–44）. Harvester Wheatsheaf.
   — Stuck 状態モデルと高い自己遷移確率の基盤となる行き詰まり・固執理論。

5. **Rabiner, L. R.**（1989）. A tutorial on hidden Markov models. *Proceedings of the IEEE, 77*(2), 257–286.
   — `CognitiveStateEngine::update()` で使用する HMM 前向きアルゴリズムの定式化。

6. **Dhakal, V., Feit, A. M., Kristensson, P. O., & Oulasvirta, A.**（2018）. Observations on typing from 136 million keystrokes. *CHI 2018*.
   — φ 正規化の参照値（β）に使用するフライトタイムと修正率の集団ベースライン値。

7. **Salthouse, T. A.**（1986）. Perceptual, cognitive, and motoric aspects of transcription typing. *Psychological Bulletin, 99*(3), 303–319.
   — フライトタイムの分解と熟練タイピングにおける予測的処理。F1・F4 特徴量設計の根拠。

8. **Apple Inc.**（2023）. *Text Input Sources Reference (Carbon)*. Apple Developer Documentation.
   — `TISCopyCurrentKeyboardInputSource`、`kTISPropertyInputSourceID`；クロスプロセス IME モード検出に使用。

---

## ライセンス

研究プロトタイプ。All rights reserved.

---

*最終更新：2026-02-28*
