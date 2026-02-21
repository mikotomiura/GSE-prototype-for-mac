use std::collections::HashMap;
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
    pub is_paused: Arc<Mutex<bool>>,
    pub backspace_streak: Arc<Mutex<u32>>,

    // 2-axis EWMA: (X = Friction, Y = Engagement)
    // α = 0.3: 新値30%、前値70%のブレンド
    axes_ewma: Arc<Mutex<(f64, f64)>>,
}

impl CognitiveStateEngine {
    pub fn new() -> Self {
        // Transition probabilities
        // FLOW -> FLOW: 0.80  (escape time 1/(1-0.80)=5s; reduced from 0.92 to prevent saturation)
        // FLOW -> INCUBATION: 0.13
        // FLOW -> STUCK: 0.07
        // INCUBATION -> FLOW: 0.12
        // INCUBATION -> INCUBATION: 0.80  (Sio & Ormerod 2009)
        // INCUBATION -> STUCK: 0.08
        // STUCK -> FLOW: 0.06
        // STUCK -> INCUBATION: 0.18
        // STUCK -> STUCK: 0.76  (Hall et al. 2024)
        let transitions = [0.80, 0.13, 0.07, 0.12, 0.80, 0.08, 0.06, 0.18, 0.76];

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
        // Penalty bin (obs=25): backspace_streak ≥ 5 → near-certain Stuck.
        // Each state's non-penalty bins sum to ≈1.0; HMM normalizes anyway.
        #[rustfmt::skip]
        let emissions: [f64; 78] = [
            // ── Flow (State 0) ─────────────────────────── non-penalty sum = 1.00
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.01, 0.02, 0.05, 0.16, 0.20,
            //  x=1            y: 0     1     2     3     4
                               0.01, 0.02, 0.05, 0.14, 0.16,
            //  x=2            y: 0     1     2     3     4
                               0.00, 0.01, 0.03, 0.06, 0.08,
            //  x=3            y: 0     1     2     3     4
                               0.00, 0.00, 0.00, 0.00, 0.00,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.00, 0.00, 0.00, 0.00, 0.00,
            //  penalty bin
                               0.00,

            // ── Incubation (State 1) ──────────────────── non-penalty sum = 1.00
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.15, 0.10, 0.04, 0.01, 0.00,
            //  x=1            y: 0     1     2     3     4
                               0.14, 0.10, 0.04, 0.01, 0.00,
            //  x=2            y: 0     1     2     3     4
                               0.10, 0.08, 0.03, 0.01, 0.00,
            //  x=3            y: 0     1     2     3     4
                               0.05, 0.04, 0.01, 0.00, 0.00,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.04, 0.03, 0.01, 0.00, 0.00,
            //  penalty bin
                               0.01,

            // ── Stuck (State 2) ───────────────── non-penalty sum = 1.00 (+0.99)
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.00, 0.00, 0.00, 0.00, 0.00,
            //  x=1            y: 0     1     2     3     4
                               0.00, 0.00, 0.00, 0.00, 0.00,
            //  x=2            y: 0     1     2     3     4
                               0.02, 0.04, 0.02, 0.00, 0.00,
            //  x=3            y: 0     1     2     3     4
                               0.10, 0.16, 0.07, 0.02, 0.00,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.16, 0.22, 0.12, 0.05, 0.02,
            //  penalty bin  (backspace streak ≥5 → near-certain Stuck)
                               0.99,
        ];

        // 初期事前確率: バランス型で開始 (Flow偏重を排除)
        let initial_probs = [0.5, 0.3, 0.2];

        Self {
            transitions: Arc::new(transitions),
            emissions: Arc::new(emissions),
            current_state_probs: Arc::new(Mutex::new(initial_probs)),
            is_paused: Arc::new(Mutex::new(false)),
            backspace_streak: Arc::new(Mutex::new(0)),
            // (0.3, 0.5) = 中立領域で初期化 (obs=7; Flow/Inc/Stuck がほぼ均等な観測ビン)
            // (0.0, 1.0) で開始すると初回更新で p_flow=1.0 に固定されるため変更
            axes_ewma: Arc::new(Mutex::new((0.3, 0.5))),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        match self.is_paused.lock() {
            Ok(mut p) => *p = paused,
            Err(poisoned) => *poisoned.into_inner() = paused,
        }
    }

    /// IME active時に強制的にFlow状態にする (Stuck表示を消す)
    /// EWMA もリセットして誤った蓄積値が残らないようにする
    pub fn force_flow_state(&self) {
        let flow_probs = [0.98, 0.01, 0.01];
        match self.current_state_probs.lock() {
            Ok(mut p) => *p = flow_probs,
            Err(poisoned) => *poisoned.into_inner() = flow_probs,
        }
        // IME切り替え時にEWMAをリセット: (低Friction, 高Engagement) = Flow領域
        match self.axes_ewma.lock() {
            Ok(mut e) => *e = (0.0, 1.0),
            Err(poisoned) => *poisoned.into_inner() = (0.0, 1.0),
        }
    }

    /// IMEポーズ中かどうかを安全に取得する
    pub fn get_paused(&self) -> bool {
        match self.is_paused.lock() {
            Ok(g) => *g,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub fn discretize_flight_time(&self, ft: f64) -> usize {
        match ft {
            t if t < 80.0 => 0,
            t if t < 120.0 => 1,
            t if t < 160.0 => 2,
            t if t < 200.0 => 3,
            t if t < 300.0 => 4,
            t if t < 400.0 => 5,
            t if t < 500.0 => 6,
            t if t < 700.0 => 7,
            t if t < 1000.0 => 8,
            t if t < 1500.0 => 9,
            _ => 10,
        }
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
    fn calculate_latent_axes(&self, features: &Features) -> (f64, f64) {
        const BETA_F1: f64 = 250.0; // 標準FT中央値 (ms)
        const BETA_F3: f64 = 0.10; // 標準修正率 (10%)
        const BETA_F4: f64 = 2.0; // 標準バースト長 (文字数)
        const BETA_F5: f64 = 3.0; // 標準ポーズ回数 (3回/30s)
        const BETA_F6: f64 = 0.15; // 標準削除後停止率 (15%)

        let phi1 = phi(features.f1_flight_time_median, BETA_F1);
        let phi3 = phi(features.f3_correction_rate, BETA_F3);
        let phi4 = phi(features.f4_burst_length, BETA_F4);
        let phi5 = phi(features.f5_pause_count, BETA_F5);
        let phi6 = phi(features.f6_pause_after_del_rate, BETA_F6);

        // X: Friction (高いほど「つまずき」)  重み合計 = 1.0
        let x = (0.30 * phi3 + 0.25 * phi6 + 0.25 * phi1 + 0.20 * phi5).clamp(0.0, 1.0);

        // Y: Engagement (高いほど「滑らかな出力」)  重み合計 = 1.0
        let y = (0.40 * phi4 + 0.35 * (1.0 - phi1) + 0.25 * (1.0 - phi5)).clamp(0.0, 1.0);

        (x, y)
    }

    /// B-5: HMM Update (2軸 Friction × Engagement、25+1ビン モデル)
    pub fn update(&self, features: &Features, vk_code: Option<u32>) {
        if self.get_paused() {
            return;
        }

        // F1がゼロの場合はデータ不足のためスキップ
        if features.f1_flight_time_median <= 0.0 {
            return;
        }

        // --- Backspace Streak Logic ---
        // 5回以上の連続Backspaceを「大きな修正＝Stuck」とする安全装置
        let streak = match self.backspace_streak.lock() {
            Ok(mut s) => {
                if vk_code == Some(0x08) {
                    // VK_BACK
                    *s += 1;
                } else if vk_code.is_some() {
                    *s = 0;
                }
                *s
            }
            Err(poisoned) => {
                let mut s = poisoned.into_inner();
                if vk_code == Some(0x08) {
                    *s += 1;
                } else if vk_code.is_some() {
                    *s = 0;
                }
                *s
            }
        };
        let apply_backspace_penalty = streak >= 5;

        let (raw_x, raw_y) = self.calculate_latent_axes(features);

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
        // ε-floor: 放射確率の最小値を保証し、単一観測で状態確率が完全に0になることを防ぐ
        // これにより p_flow=1.0 への飽和 (確率の吸収状態) を抑制する
        const EMISSION_FLOOR: f64 = 0.01;

        let mut sum_prob = 0.0;

        for j in 0..n_states {
            let mut trans_sum = 0.0;
            for i in 0..n_states {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_probs[i] * t_prob;
            }

            // 3 states × 26 bins: index = j * 26 + obs
            let e_prob = self.emissions[j * 26 + obs] + EMISSION_FLOOR;
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
    }

    pub fn get_current_state(&self) -> HashMap<CognitiveState, f64> {
        // A-3: Mutex ポイズニング対策
        let probs = match self.current_state_probs.lock() {
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
