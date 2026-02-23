# GSE-Next — Generative Struggle Engine（プロトタイプ v2）

> **Windows 11 上でキーストローク動的特徴からリアルタイムに認知状態を推定するシステム**
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
8. [IME 検出](#ime-検出)
9. [ログと分析](#ログと分析)
10. [ビルド手順](#ビルド手順)
11. [学術的参考文献](#学術的参考文献)

---

## 研究動機

ライター、プログラマー、知識労働者は **Flow（滑らかな高出力）**・**Incubation（意識的な停止と潜在的処理）**・**Stuck（認知的行き詰まり、非生産的ループ）** の三状態を交互に経験します。これらの状態をリアルタイムで把握できれば、アンビエント音楽・ナッジ・UI ディミングなど、タスクを中断しないまま認知的足場（metacognitive scaffolding）を提供するアダプティブツールが実現できます。

既存のアプローチはウェアラブルデバイス・カメラ・明示的自己報告を必要とします。本プロトタイプは OS から既に取得可能な**キーストロークタイミング情報のみ**を使用するため、追加ハードウェアなしで任意の Windows デバイスに展開可能です。

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
│                         Windows 11（Surface Pro 8）                  │
│                                                                      │
│  ┌─────────────┐   WH_KEYBOARD_LL    ┌──────────────────────────┐  │
│  │ 任意のアプリ │ ─────────────────── │  フックスレッド（Rust）   │  │
│  │ （フォアグラ │                     │  SetWindowsHookExW       │  │
│  │  ウンド）   │                     │  WinEvent IME モニター   │  │
│  └─────────────┘                     └──────────┬───────────────┘  │
│                                                 │ crossbeam channel │
│                                      ┌──────────▼───────────────┐  │
│                                      │  分析スレッド（Rust）     │  │
│                                      │                          │  │
│                                      │  FeatureExtractor        │  │
│                                      │    F1 フライトタイム中央値│  │
│                                      │    F2 フライトタイム分散 │  │
│                                      │    F3 修正率             │  │
│                                      │    F4 バースト長         │  │
│                                      │    F5 ポーズ回数         │  │
│                                      │    F6 削除後停止率       │  │
│                                      │                          │  │
│                                      │  CognitiveStateEngine    │  │
│                                      │    潜在軸（X, Y）        │  │
│                                      │    EWMA 平滑化           │  │
│                                      │    HMM 前向きアルゴリズム│  │
│                                      │    ヒステリシス EMA 層   │  │
│                                      └──────────┬───────────────┘  │
│                                                 │ Tauri IPC         │
│                                      ┌──────────▼───────────────┐  │
│                                      │  React/TS ダッシュボード  │  │
│                                      │  フローティングオーバーレイ│  │
│                                      │  霧エフェクト（Stuck 時） │  │
│                                      └──────────────────────────┘  │
│                                                 │                   │
│                                      ┌──────────▼───────────────┐  │
│                                      │  SessionLogger（Rust）   │  │
│                                      │  NDJSON → Documents/     │  │
│                                      │  GSE-sessions/           │  │
│                                      └──────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

### スレッドモデル

```
メインスレッド（Tauri イベントループ）
    │
    ├─ フックスレッド       ← WH_KEYBOARD_LL メッセージループ + WinEvent IME コールバック
    │       │ crossbeam::channel (bounded 64, ノンブロッキング送信)
    ├─ 分析スレッド         ← recv_timeout(1 s) でキーイベントと沈黙の両方で HMM 更新
    │       │ Arc<Mutex<CognitiveStateEngine>> (Tauri managed state)
    ├─ IME モニタースレッド ← 100ms ごとに is_candidate_window_open() をポーリング
    │
    └─ ロガースレッド       ← bounded channel(512) → NDJSON ファイル (BufWriter)
```

---

## フォルダ構成

```
GSE-Next/
├── analysis/
│   ├── behavioral_gt.py       # セッション後行動ルールベース GT ラベリング
│   └── hmm_sensitivity.py     # HMM パラメータ感度分析
│
├── src/                       # React / TypeScript フロントエンド
│   ├── components/
│   │   ├── Dashboard.tsx      # 状態確率バー + 霧エフェクトオーバーレイ
│   │   └── Overlay.tsx        # 透過常時最前面ウィンドウシェル
│   ├── App.tsx
│   └── main.tsx
│
├── src-tauri/                 # Rust / Tauri 2.0 バックエンド
│   ├── capabilities/
│   │   └── default.json       # Tauri 2.0 ケイパビリティ宣言
│   ├── src/
│   │   ├── analysis/
│   │   │   ├── engine.rs      # HMM エンジン + ヒステリシス層（display_probs EMA）
│   │   │   ├── features.rs    # F1–F6 特徴量抽出 + 沈黙合成
│   │   │   └── mod.rs
│   │   ├── input/
│   │   │   ├── hook.rs        # WH_KEYBOARD_LL フック + WinEvent IME 検出
│   │   │   ├── ime.rs         # ImeMonitor（EnumWindows + UIAutomation フォールバック）
│   │   │   └── mod.rs
│   │   ├── lib.rs             # Tauri セットアップ、スレッド管理、IPC コマンド
│   │   ├── logger.rs          # 非同期 NDJSON セッションロガー
│   │   ├── main.rs
│   │   └── sensors.rs         # 加速度センサー + ジオロケーター（WinRT）
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
これは暗黙的に σ = κ·β とした片側 z-スコアと等価です。

---

## HMM エンジン

### 意味的潜在軸：Cognitive Friction（認知摩擦）× Productive Silence（生産的沈黙）

6 つの正規化特徴量を 2 つの解釈可能な意味的潜在軸に射影したうえで離散化します。

```text
X（Cognitive Friction）   = 0.30·φ(F3) + 0.25·φ(F6) + 0.25·φ(F1) + 0.20·φ(F5)
Y（Productive Silence）   = 0.40·φ(F4) + 0.35·(1 − φ(F1)) + 0.25·(1 − φ(F5))
```

**Cognitive Friction（$X$）**: 「躊躇」や苦悩の深さを定量化します。Stuck 指標 $P(\text{Pause} \mid \text{Delete})$（F6 に相当）に大きなウェイトを置きます。

**Productive Silence（$Y$）**: 沈黙がどの程度生産的なバーストにつながるかを示します。価値あるDMN活性化型の熟考と単なる認知的停滞を区別します。

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
- **ε** = 0.04（放射フロア；確率吸収を防ぎ、クラスタリングを緩和）

### 遷移行列 A

| 遷移元 \ 先 | Flow | Incubation | Stuck |
| --- | --- | --- | --- |
| **Flow** | 0.80 | 0.13 | 0.07 |
| **Incubation** | 0.12 | 0.80 | 0.08 |
| **Stuck** | 0.06 | 0.18 | 0.76 |

Flow 自己遷移 0.80 → 平均滞在時間 ≈ 5 秒。
Incubation 0.80 → Sio & Ormerod（2009）の知見（熟考期間は数秒〜数分）に対応。
Stuck 0.76 → 高い固執傾向（Ohlsson, 1992）に対応。

---

## ヒステリシスと安定性修正（v2.1）

セッションログの分析から三つの病理的挙動が確認され、修正されました。

---

### 修正 ①：Cold-Start ヒステリシス（Stuck→Flow ウィンドウリセットスパイク）

**理論的注記：HSMMに対するO(1)代替手法**
標準HMMは状態継続時間分布を明示的にモデル化できません（幾何分布的な減衰を仮定）。Hidden Semi-Markov Model（HSMM）はIncubationとStuckの非幾何的な継続時間をモデル化するために理論的に最適ですが、$O(T^2)$の計算複雑度をもたらし、継続時間パラメータの推定に大量データを必要とします（$n=1$環境での過学習）。以下に導入するEMAヒステリシス層は、HSMMのオーバーヘッドなしに最小状態滞在時間を強制する$O(1)$の計算的工夫であり、エッジ推論に理想的です。

**問題：** t = 255.2 s に 30 秒ウィンドウが大量 Backspace 区間を過ぎると、削除イベントがウィンドウ外に出て新ウィンドウの特徴量が Flow に見えます。結果：`p_stuck = 0.994 → p_flow = 0.48` が 1 ミリ秒で発生。

**根本原因：** `get_current_state()` が時間的慣性のない生 HMM 信念（点推定値）を直接返していた。

**修正：** 生 HMM 信念と並行して補助確率ベクトル `display_probs` を維持し、遅い EMA で追跡します。

```
display_t = α · raw_t + (1 − α) · display_{t−1}

α = 0.25  （通常更新 → 時定数 τ ≈ 4 更新 ≈ 4 秒）
α = 0.50  （Backspace ペナルティビン → 素早い Stuck 収束）
```

`get_current_state()` は `display_probs` を返します。真の状態遷移は約 4 秒の持続的エビデンスを必要とするため、UI やログへの即時反映が防止されます。

ウィンドウリセットシナリオのシミュレーション：

| Tick | 生 p_flow | display p_stuck |
| --- | --- | --- |
| 0（リセット前） | 0.01 | **0.994** |
| 1（Flow シグナル） | 0.48 | 0.748 |
| 2 | 0.52 | 0.563 |
| 3 | 0.54 | 0.424 |
| 4 | 0.56 | 0.319 |

---

### 修正 ②：確率の離散クラスタリング（段階的天井）

**問題：** 放射フロア ε = 0.01 の状態で HMM が特定の固定点確率に収束していた。

- `p_flow` → 0.9734（フレームの 39.1% に集中）
- `p_inc` → 0.9381
- `p_stuck` → 0.9944

これらのクラスターは、特定状態が支配的な観測ビンにおいて放射確率の比率が固有の固定点を決定することで生じます。ε = 0.01 では比率が極端（例：0.20:0.01:0.01 = 20:1:1）となり、確率質量がほぼ一点に集中します。

**修正：** 放射フロアを 0.01 → **0.04** に引き上げました。これは放射ドメインにおいてラプラス加算平滑化と同等の効果をもたらします。各状態の最大到達確率は 0.88〜0.90 程度に低下し、競合状態にも意味のある確率質量が残ります。

---

### 修正 ③：Inc→Stuck 沈黙遷移（50 秒無入力で Inc に留まる）

**問題：** 長時間の沈黙（≥ 50 秒）が Incubation に分類され続けた。`make_silence_observation()` は F5（ポーズ回数）のみを設定しており、F5 単独では X（摩擦）の最大値が：

```
X_max（F5 のみ） = 0.20 · φ(F5) = 0.20 · 1.0 = 0.20 → x_bin = 1（Incubation）
```

Stuck 支配ビン（x_bin ≥ 3、X ≥ 0.60）には F3 または F6 なしには到達不可能でした。

**認知的根拠：** 長時間の出力沈黙は意図的な Incubation と意味的に等価ではありません。30 秒を超えてタイピングせずに画面を眺める行動は「熟考」よりも「詰まり」に近いと解釈できます。極端な沈黙中に摩擦を合成することは、この認知的遷移を反映するものです。

**修正：** `make_silence_observation()` に沈黙時間に応じて線形増加する合成摩擦値を追加しました。

```rust
// F6：20 秒超で開始 → 80 秒で 0.50 に到達
F6_synthetic = clamp((silence_secs − 20) / 60,  0.0, 0.50)

// F3：30 秒超で開始 → 130 秒で 0.40 に到達
F3_synthetic = clamp((silence_secs − 30) / 100, 0.0, 0.40)
```

X の時系列（F5 が φ = 1.0 に飽和、典型的 F1）：

| 沈黙時間 | F3_合成 | F6_合成 | X（摩擦） | x_bin | 領域 |
| --- | --- | --- | --- | --- | --- |
| 20 秒 | 0.00 | 0.00 | ≈ 0.20 | 1 | Incubation |
| 30 秒 | 0.00 | 0.17 | ≈ 0.30 | 1 | Incubation |
| 40 秒 | 0.10 | 0.33 | ≈ 0.52 | 2 | 境界域 |
| 50 秒 | 0.20 | 0.50 | ≈ 0.75 | 3 | **Stuck** |

EWMA 平滑化（α = 0.30）後、Stuck 観測はさらに約 5 秒かけて反映されます。ヒステリシス層と合わせると、Stuck ラベルは持続的高摩擦沈黙の約 9 秒後に確認されます。

---

## IME 検出

日本語（その他の CJK）入力では組み立て段階（ローマ字→かな変換）で生のキーイベントが最終文字に対応しません。これらのイベントを解析すると特徴量ベクトルが破損します。三つの相補的検出層を使用します。

| 層 | 手法 | 備考 |
| --- | --- | --- |
| **主** | `SetWinEventHook`（EVENT\_OBJECT\_IME\_CHANGE/SHOW/HIDE） | クロスプロセス。DLL インジェクション不要。候補リスト表示前のローマ字→かな変換段階から検出可能 |
| **副** | `EnumWindows` による "CandidateUI" / "IME" ウィンドウクラス検索 | 候補選択段階をカバー |
| **三次** | UIAutomation `GetFocusedElement` | IME ウィンドウにフォーカスがある場合のみ有効 |

**ステールフラグ回復：** WinEvent フラグが立っているが副・三次の両方が候補ウィンドウを確認できない場合、フラグをリセットして永続的な分析停止を防ぎます。

**MSCTFIME UI は明示的に除外：** このクラスは TSF 言語バー（タスクバーの A/あ インジケーター）に属し、日本語 IME が読み込まれている場合は常時表示されます。含めると恒久的な誤検知が発生します。

グローバル TSF フック（`ITfThreadMgr`）は使用しません — プロセス境界を越えると UIPI によってブロックされます。

---

## ログと分析

セッションごとにタイムスタンプ付き NDJSON ファイルが生成されます。

```
%USERPROFILE%\Documents\GSE-sessions\gse_YYYYMMDD_HHMMSS.ndjson
```

レコードタイプ：

```jsonc
// セッションメタデータ
{"type":"meta","session_start":1740000000000}

// 生キーストロークイベント
{"type":"key","t":1740000001234,"vk":65,"press":true}

// 特徴量スナップショット + HMM 状態確率（キー押下または沈黙ティックごと）
{"type":"feat","t":1740000001235,
 "f1":145.20,"f2":312.00,"f3":0.0800,"f4":6.50,"f5":1.0,"f6":0.0000,
 "p_flow":0.7123,"p_inc":0.2100,"p_stuck":0.0777}

{"type":"meta","session_end":1740000060000}
```

### セッション後グラウンドトゥルースラベリング

```bash
python analysis/behavioral_gt.py gse_YYYYMMDD_HHMMSS.ndjson
```

30 秒スライディングウィンドウ（1 秒ステップ）でラベルを付与します。

| ラベル | 行動ルール |
| --- | --- |
| **FLOW** | median(FT) < 200 ms かつ correction\_rate < 0.15 かつ STUCK/INC でない |
| **INCUBATION** | Pause(≥2 s) → Burst(≥5 文字 FT<200 ms) → diff\_chars ≥ 3（30 秒以内） |
| **STUCK** | 「Burst(≤3 文字) → Delete(≥1) → Pause(≥2 s)」のループ ≥ 3 回（60 秒内）かつ diff\_chars ≤ 0 |
| **UNKNOWN** | 条件を満たさない、または複数ラベルが競合 |

---

## ビルド手順

### 前提条件

| ツール | バージョン |
| --- | --- |
| Rust | 1.77+（`rustup update stable`） |
| Node.js | 20+ |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |

### 開発実行

```bash
cd GSE-Next
npm install
npm run tauri dev
```

### リリースビルド

```bash
npm run tauri build
# インストーラー: src-tauri/target/release/bundle/
```

### セッション後分析

```bash
python analysis/behavioral_gt.py "%USERPROFILE%\Documents\GSE-sessions\gse_YYYYMMDD_HHMMSS.ndjson"
```

---

## 学術的参考文献

1. **Csikszentmihalyi, M.**（1990）. *Flow: The Psychology of Optimal Experience*. Harper & Row.
   — Flow 認知状態の定義とその行動的相関の基盤。

2. **Csikszentmihalyi, M.**（1996）. *Creativity: Flow and the Psychology of Discovery and Invention*. HarperCollins.
   — 創造的・生成的ライティング課題へのフロー理論の拡張。

3. **Sio, U. N., & Ormerod, T. C.**（2009）. Does incubation enhance problem solving? A meta-analytic review. *Psychological Bulletin, 135*(1), 94–120.
   — Incubation 状態の自己遷移確率（0.80）と Pause→Burst 行動シグネチャの実証的根拠。

4. **Ohlsson, S.**（1992）. Information-processing explanations of insight and related phenomena. In M. T. Keane & K. J. Gilhooly（Eds.）, *Advances in the Psychology of Thinking*（pp. 1–44）. Harvester Wheatsheaf.
   — Stuck 状態モデルと高い自己遷移確率の基盤となる行き詰まり・固執理論。

5. **Rabiner, L. R.**（1989）. A tutorial on hidden Markov models and selected applications in speech recognition. *Proceedings of the IEEE, 77*(2), 257–286.
   — `CognitiveStateEngine::update()` で使用する HMM 前向きアルゴリズムの定式化。

6. **Dhakal, V., Feit, A. M., Kristensson, P. O., & Oulasvirta, A.**（2018）. Observations on typing from 136 million keystrokes. *Proceedings of CHI 2018*.
   — φ 正規化の参照値（β）に使用するフライトタイムと修正率の集団ベースライン値。

7. **Salthouse, T. A.**（1986）. Perceptual, cognitive, and motoric aspects of transcription typing. *Psychological Bulletin, 99*(3), 303–319.
   — フライトタイムの分解と熟練タイピングにおける予測的処理。F1・F4 特徴量設計の根拠。

8. **Microsoft Corporation.**（2023）. *WinEvent Hooks*. Windows Developer Documentation（MSDN）.
   — `SetWinEventHook`、`EVENT_OBJECT_IME_CHANGE/SHOW/HIDE` 定数、および `WINEVENT_OUTOFCONTEXT` フラグ。DLL インジェクションなしのクロスプロセス IME 検出に使用。

---

## ライセンス

研究プロトタイプ。All rights reserved.

---

*最終更新：2026-02-22*
