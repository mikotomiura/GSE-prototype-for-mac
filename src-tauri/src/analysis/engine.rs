use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::analysis::features::{Features, phi};

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
    emissions: Arc<[f64; 33]>,

    current_state_probs: Arc<Mutex<[f64; 3]>>,
    pub is_paused: Arc<Mutex<bool>>,
}

impl CognitiveStateEngine {
    pub fn new() -> Self {
        // B-4: 設計書通りのバタつきを抑えた遷移行列
        // Flow -> Flow (0.92), Incubation (0.07), Stuck (0.01)
        // Incubation -> Flow (0.10), Incubation (0.82), Stuck (0.08)
        // Stuck -> Flow (0.02), Incubation (0.13), Stuck (0.85)
        let transitions = [
            0.92, 0.07, 0.01,
            0.10, 0.82, 0.08,
            0.02, 0.13, 0.85,
        ];

        // Emissions (B) 3x11
        // S_stuck=[0,1] を 11ビンにマッピング
        // Flow (State 0): 低観測値 (S_stuck が低い) にバイアス
        // Incubation (State 1): 中間観測値にバイアス
        // Stuck (State 2): 高観測値 (S_stuck が高い) にバイアス
        #[rustfmt::skip]
        let emissions = [
            // Flow
            0.3, 0.3, 0.2, 0.1, 0.05, 0.02, 0.01, 0.01, 0.005, 0.005, 0.0,
            // Incubation
            0.01, 0.02, 0.05, 0.1, 0.2, 0.2, 0.2, 0.1, 0.05, 0.04, 0.03,
            // Stuck
            0.0, 0.005, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.3, 0.3, 0.01,
        ];

        // Initial Priors
        let initial_probs = [0.5, 0.4, 0.1];

        Self {
            transitions: Arc::new(transitions),
            emissions: Arc::new(emissions),
            current_state_probs: Arc::new(Mutex::new(initial_probs)),
            is_paused: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Ok(mut p) = self.is_paused.lock() {
            *p = paused;
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

    /// B-3: S_stuck (Stuckスコア) を算出する
    /// 重み: w1=0.30(F1), w2=0.25(F3), w3=0.25(F6), w4=0.20(F4)
    fn calculate_s_stuck(&self, features: &Features) -> f64 {
        // 個人ベースライン (M1段階では固定値)
        const BETA_F1: f64 = 150.0; // 標準FT中央値 (ms)
        const BETA_F3: f64 = 0.05;  // 標準修正率 (5%)
        const BETA_F4: f64 = 5.0;   // 標準バースト長 (文字数)
        const BETA_F6: f64 = 0.10;  // 標準削除後停止率 (10%)

        let phi1 = phi(features.f1_flight_time_median, BETA_F1);
        let phi3 = phi(features.f3_correction_rate, BETA_F3);
        // F4: バースト長が長い = フロー → (1 - phi) でStuckへの寄与を逆転
        let phi4_inv = 1.0 - phi(features.f4_burst_length, BETA_F4);
        let phi6 = phi(features.f6_pause_after_del_rate, BETA_F6);

        0.30 * phi1 + 0.25 * phi3 + 0.25 * phi6 + 0.20 * phi4_inv
    }

    /// B-5: 引数を Features に変更し、S_stuck ベースの観測値でHMMを更新する
    pub fn update(&self, features: &Features) {
        if self.get_paused() {
            return;
        }

        // F1がゼロの場合はデータ不足のためスキップ
        if features.f1_flight_time_median <= 0.0 {
            return;
        }

        let s_stuck = self.calculate_s_stuck(features);

        // S_stuck [0, 1] → 観測ビン [0, 10]
        let obs = ((s_stuck * 10.0) as usize).min(10);

        // A-3: Mutex ポイズニング対策
        let mut current = match self.current_state_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let old_probs = *current;
        let mut new_probs = [0.0; 3];
        let n_states = 3;

        // Forward Algorithm Step
        // new_prob[j] = (Σ_i old_prob[i] * trans[i->j]) * emission[j][obs]
        let mut sum_prob = 0.0;

        for j in 0..n_states {
            let mut trans_sum = 0.0;
            for i in 0..n_states {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_probs[i] * t_prob;
            }

            let e_prob = self.emissions[j * 11 + obs];
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
