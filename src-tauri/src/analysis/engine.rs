use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::analysis::features::{phi, Features};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CognitiveState {
    Flow,
    Incubation,
    Stuck,
}

#[derive(Clone)]
pub struct CognitiveStateEngine {
    // Manual HMM parameters
    transitions: Arc<[f64; 9]>,

    // 3 states × 26 observation bins
    //   obs = x_bin * 5 + y_bin  (0..24 natural bins)
    //   obs = 25                  (backspace streak penalty bin)
    //
    // X-axis (Friction):   0 = low friction  … 4 = high friction
    // Y-axis (Engagement): 0 = low engagement … 4 = high engagement
    emissions: Arc<[f64; 78]>,

    current_state_probs: Arc<Mutex<[f64; 3]>>,
    pub is_paused: Arc<AtomicBool>,
    pub backspace_streak: Arc<AtomicU32>,
    // ペナルティ保留フラグ: streak >= 8 に達した瞬間にtrueになり、
    // update() で消費されるまで保持される。
    // register_keystroke() で streak をリセットしてもペナルティは取りこぼさない。
    has_pending_penalty: Arc<AtomicBool>,

    // 2-axis EWMA: (X = Friction, Y = Engagement)
    // α = 0.3: 新値30%、前値70%のブレンド
    axes_ewma: Arc<Mutex<(f64, f64)>>,

    // Hysteresis layer: slow EMA of reported probabilities.
    // Prevents instant state flips (e.g. Cold-Start after window reset).
    // α = 0.40 for normal updates (~2.5s time-constant).
    // α = 0.60 for backspace-penalty bin (faster Stuck response).
    display_probs: Arc<Mutex<[f64; 3]>>,
}

impl CognitiveStateEngine {
    pub fn new() -> Self {
        // Transition probabilities
        // FLOW -> FLOW: 0.75  (escape time 1/(1-0.75)=4s; reduced from 0.80 to mitigate Flow Gravity)
        // FLOW -> INCUBATION: 0.17
        // FLOW -> STUCK: 0.08
        // INCUBATION -> FLOW: 0.12
        // INCUBATION -> INCUBATION: 0.80  (Sio & Ormerod 2009)
        // INCUBATION -> STUCK: 0.08
        // STUCK -> FLOW: 0.06
        // STUCK -> INCUBATION: 0.18
        // STUCK -> STUCK: 0.76  (Hall et al. 2024)
        let transitions = [0.75, 0.17, 0.08, 0.12, 0.80, 0.08, 0.06, 0.18, 0.76];

        // Emissions B: 3 states × 26 bins
        //
        // Grid layout: obs = x_bin * 5 + y_bin
        //
        //        x→  0(lo F)  1      2      3      4(hi F)
        //  y↓
        //  0(lo E)   [0]     [5]   [10]   [15]   [20]
        //  1         [1]     [6]   [11]   [16]   [21]
        //  2         [2]     [7]   [12]   [17]   [22]
        //  3         [3]     [8]   [13]   [18]   [23]
        //  4(hi E)   [4]     [9]   [14]   [19]   [24]
        //  penalty   [25]
        //
        // Flow:       peaks at low Friction (x=0,1) × high Engagement (y=3,4)
        // Incubation: peaks at low-mid Friction (x=0,1,2) × low Engagement (y=0,1)
        // Stuck:      peaks at high Friction (x=3,4) × low Engagement (y=0,1)
        //
        // Penalty bin (obs=25): backspace_streak ≥ 8 → near-certain Stuck.
        //
        // 全ビンに最小値 0.01 を設定し、確率の完全消滅を防止。
        // 旧実装の EMISSION_FLOOR (+0.05 一律加算) を廃止:
        //   - 旧: ペナルティビン Stuck:Flow 比 = 1.04/0.05 = 20.8x（鈍い応答）
        //   - 新: ペナルティビン Stuck:Flow 比 = 0.99/0.01 = 99x（鋭い応答）
        // EWMA (α=0.3) とヒステリシス層 (α=0.25/0.50) が安定性を維持する。
        #[rustfmt::skip]
        let emissions: [f64; 78] = [
            // ── Flow (State 0) ─────────────────────────── non-penalty sum ≈ 0.97
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.01, 0.02, 0.05, 0.12, 0.14,
            //  x=1            y: 0     1     2     3     4
                               0.01, 0.02, 0.05, 0.12, 0.13,
            //  x=2            y: 0     1     2     3     4
                               0.01, 0.01, 0.03, 0.06, 0.08,
            //  x=3            y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  penalty bin
                               0.01,

            // ── Incubation (State 1) ──────────────────── non-penalty sum ≈ 1.12
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.15, 0.10, 0.04, 0.03, 0.02,
            //  x=1            y: 0     1     2     3     4
                               0.14, 0.10, 0.04, 0.03, 0.02,
            //  x=2            y: 0     1     2     3     4
                               0.10, 0.08, 0.03, 0.01, 0.01,
            //  x=3            y: 0     1     2     3     4
                               0.05, 0.04, 0.01, 0.01, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.04, 0.03, 0.01, 0.01, 0.01,
            //  penalty bin
                               0.01,

            // ── Stuck (State 2) ─────────────────────────
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=1            y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=2            y: 0     1     2     3     4
                               0.02, 0.04, 0.02, 0.01, 0.01,
            //  x=3            y: 0     1     2     3     4
                               0.10, 0.16, 0.07, 0.02, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.16, 0.22, 0.12, 0.05, 0.02,
            //  penalty bin  (backspace streak ≥5 → near-certain Stuck)
                               0.99,
        ];

        // 初期事前確率: Flow優勢で開始 (セッション開始直後のフリッカー防止)
        // 最初の1-2秒はサイレンス観測(f1=2000)が流入し Incubation 方向に引っ張るが、
        // Flow優勢の事前確率がこの過渡的ノイズを吸収する。
        // HMM は実際の打鍵データで数秒以内に実態に収束する。
        let initial_probs = [0.80, 0.15, 0.05];

        Self {
            transitions: Arc::new(transitions),
            emissions: Arc::new(emissions),
            current_state_probs: Arc::new(Mutex::new(initial_probs)),
            is_paused: Arc::new(AtomicBool::new(false)),
            backspace_streak: Arc::new(AtomicU32::new(0)),
            has_pending_penalty: Arc::new(AtomicBool::new(false)),
            // (0.1, 0.8) = Flow領域で初期化 (obs=4; Flow優勢ビン)
            // 初期確率 [0.80, 0.15, 0.05] と整合的な開始位置。
            // セッション開始直後のサイレンス観測(f1=2000)による過渡ノイズを吸収する。
            axes_ewma: Arc::new(Mutex::new((0.1, 0.8))),
            // display_probs は initial_probs と同値で初期化
            display_probs: Arc::new(Mutex::new(initial_probs)),
        }
    }

    /// HMM確率・EWMA・backspace_streakを初期値に戻す。
    /// セッション開始時に呼び出され、前回セッションの状態をリセットする。
    pub fn reset(&self) {
        let initial_probs = [0.80, 0.15, 0.05];

        match self.current_state_probs.lock() {
            Ok(mut p) => *p = initial_probs,
            Err(poisoned) => *poisoned.into_inner() = initial_probs,
        }
        match self.display_probs.lock() {
            Ok(mut p) => *p = initial_probs,
            Err(poisoned) => *poisoned.into_inner() = initial_probs,
        }
        match self.axes_ewma.lock() {
            Ok(mut e) => *e = (0.1, 0.8),
            Err(poisoned) => *poisoned.into_inner() = (0.1, 0.8),
        }
        self.backspace_streak.store(0, Ordering::Relaxed);
        self.has_pending_penalty.store(false, Ordering::Release);
        self.is_paused.store(false, Ordering::Release);
    }

    pub fn set_paused(&self, paused: bool) {
        self.is_paused.store(paused, Ordering::Release);
    }

    /// IME active時に強制的にFlow状態にする (Stuck表示を消す)
    /// EWMA もリセットして誤った蓄積値が残らないようにする
    pub fn force_flow_state(&self) {
        let flow_probs = [0.98, 0.01, 0.01];
        match self.current_state_probs.lock() {
            Ok(mut p) => *p = flow_probs,
            Err(poisoned) => *poisoned.into_inner() = flow_probs,
        }
        // display_probs も即座にリセット (ヒステリシス層もクリア)
        match self.display_probs.lock() {
            Ok(mut p) => *p = flow_probs,
            Err(poisoned) => *poisoned.into_inner() = flow_probs,
        }
        // IME切り替え時にEWMAをリセット: (低Friction, 高Engagement) = Flow領域
        match self.axes_ewma.lock() {
            Ok(mut e) => *e = (0.0, 1.0),
            Err(poisoned) => *poisoned.into_inner() = (0.0, 1.0),
        }
    }

    /// 全キー押下時に呼び出し、Backspaceストリークをリアルタイムでカウントする。
    /// 1Hz gate の外側（全打鍵）で呼ぶことで、高速Backspace連打を正確に検知する。
    ///
    /// streak >= 8 に達した時点で has_pending_penalty フラグを立てる。
    /// 非BSキーで streak がリセットされても、フラグは update() で消費されるまで保持される。
    /// これにより「BS×6 → 即Enter」のようなケースでもペナルティを取りこぼさない。
    pub fn register_keystroke(&self, vk_code: u32) {
        if vk_code == 0x08 {
            let new_streak = self.backspace_streak.fetch_add(1, Ordering::Relaxed) + 1;
            if new_streak >= 8 {
                self.has_pending_penalty.store(true, Ordering::Release);
            }
        } else {
            self.backspace_streak.store(0, Ordering::Relaxed);
        }
    }

    /// IMEポーズ中かどうかを安全に取得する
    pub fn get_paused(&self) -> bool {
        self.is_paused.load(Ordering::Acquire)
    }

    /// X軸 (Friction / 摩擦) と Y軸 (Engagement / 没入度) を算出する。
    /// 返値はそれぞれ [0.0, 1.0] にクランプ済み。
    ///
    /// X (Friction) — 高いほど「つまずき」を表す。重み合計 = 1.0
    ///   0.30 × φ(F3: 修正率)
    ///   0.25 × φ(F6: 削除後停止率)
    ///   0.25 × φ(F1: Flight Time)
    ///   0.20 × φ(F5: ポーズ回数)
    ///
    /// Y (Engagement) — 高いほど「滑らかな出力」を表す。重み合計 = 1.0
    ///   0.40 × φ(F4: バースト長)
    ///   0.35 × (1 − φ(F1))   … 短いFT = 高エンゲージ
    ///   0.25 × (1 − φ(F5))   … 少ないポーズ = 高エンゲージ
    ///
    /// # Context-specific β (dual baseline)
    ///
    /// `ime_open = true`  → β_writing: Japanese typing baseline.
    ///   Slower flight times and shorter bursts are normal during Japanese composition.
    ///   Normalizing against a tighter baseline prevents systematic Flow over-detection
    ///   when the user is simply typing romaji at their natural Japanese pace.
    ///
    /// `ime_open = false` → β_coding: Alphanumeric/coding baseline.
    ///   Faster flight times and longer bursts are the reference for fluent coding.
    ///   A coder who is "slow" relative to their coding norm signals friction correctly.
    ///
    /// Both sets are population-level estimates; future work can adapt them per-session
    /// using EWMA updates (update only the matching β when IME state is known).
    fn calculate_latent_axes(&self, features: &Features, ime_open: bool) -> (f64, f64) {
        // β_coding: reference values for alphanumeric / coding input
        // Source: Dhakal et al. (2018) general population medians, coding-adjusted.
        const BETA_CODING_F1: f64 = 150.0; // Flight Time median (ms)
        const BETA_CODING_F3: f64 = 0.06;  // Correction rate (6%)
        const BETA_CODING_F4: f64 = 5.0;   // Burst length (chars)
        const BETA_CODING_F5: f64 = 2.0;   // Pause count (per 30 s)
        const BETA_CODING_F6: f64 = 0.08;  // Pause-after-delete rate

        // β_writing: reference values for Japanese IME writing input
        // Japanese romaji→kana composition is slower than direct ASCII entry.
        // Correction pattern differs: kanji selection uses arrow keys, not Backspace.
        const BETA_WRITING_F1: f64 = 220.0; // Flight Time median (ms) — slower
        const BETA_WRITING_F3: f64 = 0.08;  // Correction rate — slightly lower (IME handles errors)
        const BETA_WRITING_F4: f64 = 2.0;   // Burst length — shorter (composition in segments)
        const BETA_WRITING_F5: f64 = 4.0;   // Pause count — more pauses for thinking/selection
        const BETA_WRITING_F6: f64 = 0.12;  // Pause-after-delete rate

        let (beta_f1, beta_f3, beta_f4, beta_f5, beta_f6) = if ime_open {
            (BETA_WRITING_F1, BETA_WRITING_F3, BETA_WRITING_F4, BETA_WRITING_F5, BETA_WRITING_F6)
        } else {
            (BETA_CODING_F1, BETA_CODING_F3, BETA_CODING_F4, BETA_CODING_F5, BETA_CODING_F6)
        };

        let phi1 = phi(features.f1_flight_time_median, beta_f1);
        let phi3 = phi(features.f3_correction_rate, beta_f3);
        let phi4 = phi(features.f4_burst_length, beta_f4);
        let phi5 = phi(features.f5_pause_count, beta_f5);
        let phi6 = phi(features.f6_pause_after_del_rate, beta_f6);

        // X: Friction (高いほど「つまずき」)  重み合計 = 1.0
        let x = (0.30 * phi3 + 0.25 * phi6 + 0.25 * phi1 + 0.20 * phi5).clamp(0.0, 1.0);

        // Y: Engagement (高いほど「滑らかな出力」)  重み合計 = 1.0
        let y = (0.40 * phi4 + 0.35 * (1.0 - phi1) + 0.25 * (1.0 - phi5)).clamp(0.0, 1.0);

        (x, y)
    }

    /// B-5: HMM Update (2軸 Friction × Engagement、25+1ビン モデル)
    ///
    /// `ime_open`: true = Japanese IME is in Japanese-input mode (あ/カ).
    /// Used to select context-appropriate β values in `calculate_latent_axes`.
    pub fn update(&self, features: &Features, ime_open: bool) {
        if self.get_paused() {
            return;
        }

        // F1がゼロ（データ不足）の場合はスキップ。
        // ただし保留中のペナルティがある場合は処理を続行し、ペナルティビンを適用する。
        if features.f1_flight_time_median <= 0.0
            && !self.has_pending_penalty.load(Ordering::Acquire)
        {
            return;
        }

        // --- Backspace Streak Logic ---
        // register_keystroke() が streak >= 8 到達時に has_pending_penalty を true に設定済み。
        // ここではフラグを消費（swap false）し、ペナルティビン（obs=25）を適用する。
        // streak のリセットは register_keystroke 側で非BSキー入力時に行われるため、
        // ここでは streak をリセットしない（二重リセット防止）。
        let apply_backspace_penalty = self.has_pending_penalty.swap(false, Ordering::AcqRel);

        let (raw_x, raw_y) = self.calculate_latent_axes(features, ime_open);

        // EWMA平滑化 (α = 0.3): 各軸を独立に平滑化
        // s_t = 0.3 * raw_t + 0.7 * s_{t-1}
        let (x, y) = match self.axes_ewma.lock() {
            Ok(mut ewma) => {
                ewma.0 = 0.3 * raw_x + 0.7 * ewma.0;
                ewma.1 = 0.3 * raw_y + 0.7 * ewma.1;
                (ewma.0, ewma.1)
            }
            Err(poisoned) => {
                let mut ewma = poisoned.into_inner();
                ewma.0 = 0.3 * raw_x + 0.7 * ewma.0;
                ewma.1 = 0.3 * raw_y + 0.7 * ewma.1;
                (ewma.0, ewma.1)
            }
        };

        // (X, Y) → 5×5 グリッド → 観測インデックス [0..24]
        let x_bin = (x * 5.0).floor().min(4.0) as usize;
        let y_bin = (y * 5.0).floor().min(4.0) as usize;
        let mut obs = x_bin * 5 + y_bin;

        // Backspace Streak Override → ペナルティビン (obs=25)
        // 高Friction・低Engagementの極端ケースとして強制マップ
        if apply_backspace_penalty {
            obs = 25;
        }

        // A-3: Mutex ポイズニング対策
        let mut current = match self.current_state_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let old_probs = *current;
        let mut new_probs = [0.0; 3];
        let n_states = 3;

        // Forward Algorithm Step
        // 放射確率は emission テーブルに最小値 0.01 を組み込み済み。
        // 旧 EMISSION_FLOOR (+0.05 一律加算) は廃止。
        let mut sum_prob = 0.0;

        for j in 0..n_states {
            let mut trans_sum = 0.0;
            for i in 0..n_states {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_probs[i] * t_prob;
            }

            // 3 states × 26 bins: index = j * 26 + obs
            let e_prob = self.emissions[j * 26 + obs];
            new_probs[j] = trans_sum * e_prob;
            sum_prob += new_probs[j];
        }

        // Normalize
        if sum_prob > 0.0 {
            for j in 0..n_states {
                new_probs[j] /= sum_prob;
            }
        }
        // 合計が0になった場合は以前の確率を維持する (フォールバック)

        *current = new_probs;

        // ── Hysteresis Layer ──────────────────────────────────────────────
        // display_probs に EMA を適用し、ウィンドウリセット時の
        // Cold-Start 瞬間遷移 (Stuck→Flow in 1ms) を防ぐ。
        //
        // α=0.40 (通常): 時定数 ≈ 2.5 更新 ≈ 2.5 秒
        //   emission floor 削除 (0.05→0.01) で HMM 応答が鋭くなったため、
        //   旧 α=0.25 (4秒) から引き上げて表示の追従速度を改善。
        // α=0.60 (ペナルティ): Backspace 連続時は素早く Stuck に収束
        let display_alpha = if apply_backspace_penalty { 0.60 } else { 0.40 };

        let mut display = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut disp_sum = 0.0;
        for i in 0..n_states {
            display[i] = display_alpha * new_probs[i] + (1.0 - display_alpha) * display[i];
            disp_sum += display[i];
        }
        if disp_sum > 0.0 {
            for v in display.iter_mut() {
                *v /= disp_sum;
            }
        }
    }

    /// 無入力期間（サイレンス）用のHMM更新。
    /// 通常の `update()` と異なり、EWMA を更新しない。
    /// これにより、無入力時にEWMAが高Friction方向にドリフトすることを防ぐ。
    /// HMMの遷移確率による状態更新は行うため、長時間無入力→Incubation/Stuck 検出は維持される。
    pub fn update_silence(&self, features: &Features, ime_open: bool) {
        if self.get_paused() {
            return;
        }

        let (raw_x, raw_y) = self.calculate_latent_axes(features, ime_open);

        // EWMA を更新せず、現在の EWMA 値を読み取るのみ
        let (x, y) = match self.axes_ewma.lock() {
            Ok(ewma) => *ewma,
            Err(poisoned) => *poisoned.into_inner(),
        };

        // サイレンス観測のビン計算には現在のEWMA位置を使用
        // （raw値はサイレンスの極端値なので、EWMA位置の方がHMM観測として適切）
        let _ = (raw_x, raw_y); // raw値は使用しない（ドリフト防止）
        let x_bin = (x * 5.0).floor().min(4.0) as usize;
        let y_bin = (y * 5.0).floor().min(4.0) as usize;
        let obs = x_bin * 5 + y_bin;

        // サイレンス中はペナルティビンを適用しない

        let mut current = match self.current_state_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let old_probs = *current;
        let mut new_probs = [0.0; 3];
        let n_states = 3;

        let mut sum_prob = 0.0;
        for j in 0..n_states {
            let mut trans_sum = 0.0;
            for i in 0..n_states {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_probs[i] * t_prob;
            }
            let e_prob = self.emissions[j * 26 + obs];
            new_probs[j] = trans_sum * e_prob;
            sum_prob += new_probs[j];
        }

        if sum_prob > 0.0 {
            for j in 0..n_states {
                new_probs[j] /= sum_prob;
            }
        }

        *current = new_probs;

        // Hysteresis Layer (通常のα=0.40)
        let mut display = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut disp_sum = 0.0;
        for i in 0..n_states {
            display[i] = 0.40 * new_probs[i] + 0.60 * display[i];
            disp_sum += display[i];
        }
        if disp_sum > 0.0 {
            for v in display.iter_mut() {
                *v /= disp_sum;
            }
        }
    }

    pub fn get_current_state(&self) -> HashMap<CognitiveState, f64> {
        // display_probs (ヒステリシス層) を返す。
        // 生の current_state_probs は瞬間値; display_probs は遅い EMA により
        // 短期スパイクを平滑化した値。UI・ログはこちらを使用する。
        let probs = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut map = HashMap::new();
        map.insert(CognitiveState::Flow, probs[0]);
        map.insert(CognitiveState::Incubation, probs[1]);
        map.insert(CognitiveState::Stuck, probs[2]);
        map
    }
}
