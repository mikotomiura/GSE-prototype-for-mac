use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub vk_code: u32,
    pub timestamp: u64, // ms
    pub is_press: bool,
}

/// B-1: 6特徴量を格納する構造体 (設計書 01_sensing.md)
#[derive(Debug, Clone)]
pub struct Features {
    /// F1: Flight Time 中央値 (ms)
    pub f1_flight_time_median: f64,
    /// F2: Flight Time 分散
    pub f2_flight_time_variance: f64,
    /// F3: 修正率 = (BS + Del) / 全キー数
    pub f3_correction_rate: f64,
    /// F4: バースト長 = 連続FT<200ms の平均文字数
    pub f4_burst_length: f64,
    /// F5: ポーズ回数 = 2秒以上の無入力回数
    pub f5_pause_count: f64,
    /// F6: 削除後停止率 = BS/Del直後2秒以上停止する割合
    pub f6_pause_after_del_rate: f64,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            f1_flight_time_median: 0.0,
            f2_flight_time_variance: 0.0,
            f3_correction_rate: 0.0,
            f4_burst_length: 0.0,
            f5_pause_count: 0.0,
            f6_pause_after_del_rate: 0.0,
        }
    }
}

/// B-2: 個人ベースライン正規化関数 φ(x, β) = clamp((x − β) / (κ · β), 0.0, 1.0)
/// κ = 2.0
pub fn phi(x: f64, beta: f64) -> f64 {
    const KAPPA: f64 = 2.0;
    if beta <= 0.0 {
        return 0.0;
    }
    ((x - beta) / (KAPPA * beta)).clamp(0.0, 1.0)
}

// Virtual key codes
const VK_BACK: u32 = 0x08;
const VK_DELETE: u32 = 0x2E;

pub struct FeatureExtractor {
    buffer: VecDeque<InputEvent>,
    capacity: usize,
    last_release_time: Option<u64>,
    flight_times: VecDeque<u64>, // Store recent flight times for median calc
}

impl FeatureExtractor {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            last_release_time: None,
            flight_times: VecDeque::with_capacity(capacity), // Keep same size roughly
        }
    }

    pub fn process_event(&mut self, event: InputEvent) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(event);

        if event.is_press {
            if let Some(release_time) = self.last_release_time {
                if event.timestamp >= release_time {
                    let flight_time = event.timestamp - release_time;
                    // Filter outliers (e.g. > 2000ms is likely a pause, not typing rhythm)
                    if flight_time < 2000 {
                        self.add_flight_time(flight_time);
                    }
                }
            }
        } else {
            self.last_release_time = Some(event.timestamp);
        }
    }

    fn add_flight_time(&mut self, ft: u64) {
        if self.flight_times.len() >= self.capacity {
            self.flight_times.pop_front();
        }
        self.flight_times.push_back(ft);
    }

    // [DO NOT MODIFY] 中央値計算 - 現状の実装は正しい
    pub fn calculate_flight_time_median(&self) -> f64 {
        if self.flight_times.is_empty() {
            return 0.0;
        }

        let mut sorted: Vec<u64> = self.flight_times.iter().cloned().collect();
        sorted.sort_unstable();

        let len = sorted.len();
        if len % 2 == 0 {
            let mid1 = sorted[len / 2 - 1];
            let mid2 = sorted[len / 2];
            (mid1 as f64 + mid2 as f64) / 2.0
        } else {
            sorted[len / 2] as f64
        }
    }

    /// B-1: 直近30秒のバッファから6特徴量を算出する
    pub fn calculate_features(&self) -> Features {
        if self.buffer.is_empty() {
            return Features::default();
        }

        let last_ts = self.buffer.back().unwrap().timestamp;
        let cutoff = last_ts.saturating_sub(30_000);

        // 直近30秒のイベントを収集
        let events: Vec<&InputEvent> = self.buffer.iter()
            .filter(|e| e.timestamp >= cutoff)
            .collect();

        if events.is_empty() {
            return Features::default();
        }

        // --- F1: Flight Time 中央値 (既存メソッドを利用) ---
        let f1 = self.calculate_flight_time_median();

        // --- ウィンドウ内のフライトタイムを再計算 (F2用) ---
        let mut window_fts: Vec<f64> = Vec::new();
        let mut last_release: Option<u64> = None;
        for event in &events {
            if event.is_press {
                if let Some(rel) = last_release {
                    if event.timestamp >= rel {
                        let ft = (event.timestamp - rel) as f64;
                        if ft < 2000.0 {
                            window_fts.push(ft);
                        }
                    }
                }
            } else {
                last_release = Some(event.timestamp);
            }
        }

        // --- F2: Flight Time 分散 ---
        let f2 = if window_fts.len() > 1 {
            let mean = window_fts.iter().sum::<f64>() / window_fts.len() as f64;
            window_fts.iter().map(|ft| (ft - mean).powi(2)).sum::<f64>()
                / window_fts.len() as f64
        } else {
            0.0
        };

        // --- キー押下イベントのみ抽出 ---
        let press_events: Vec<&&InputEvent> = events.iter()
            .filter(|e| e.is_press)
            .collect();

        let total_keys = press_events.len();

        // --- F3: 修正率 = (BS + Del) / 全キー押下数 ---
        let correction_keys = press_events.iter()
            .filter(|e| e.vk_code == VK_BACK || e.vk_code == VK_DELETE)
            .count();

        let f3 = if total_keys > 0 {
            correction_keys as f64 / total_keys as f64
        } else {
            0.0
        };

        // --- F4: バースト長 = 連続FT<200ms のチャンクの平均文字数 ---
        let mut burst_lengths: Vec<usize> = Vec::new();
        let mut current_burst: usize = 0;
        let mut last_rel_for_burst: Option<u64> = None;

        for event in &events {
            if event.is_press {
                if let Some(rel) = last_rel_for_burst {
                    let ft = event.timestamp.saturating_sub(rel);
                    if ft < 200 {
                        current_burst += 1;
                    } else {
                        if current_burst > 0 {
                            burst_lengths.push(current_burst);
                        }
                        current_burst = 1;
                    }
                } else {
                    current_burst = 1;
                }
            } else {
                last_rel_for_burst = Some(event.timestamp);
            }
        }
        if current_burst > 0 {
            burst_lengths.push(current_burst);
        }

        let f4 = if !burst_lengths.is_empty() {
            burst_lengths.iter().sum::<usize>() as f64 / burst_lengths.len() as f64
        } else {
            0.0
        };

        // --- F5: ポーズ回数 = 連続キー押下間で2秒以上の間隔の数 ---
        let press_ts: Vec<u64> = events.iter()
            .filter(|e| e.is_press)
            .map(|e| e.timestamp)
            .collect();

        let f5 = press_ts.windows(2)
            .filter(|w| w[1].saturating_sub(w[0]) >= 2000)
            .count() as f64;

        // --- F6: 削除後停止率 = BS/Del直後に2秒以上停止する割合 ---
        let del_count = press_events.iter()
            .filter(|e| e.vk_code == VK_BACK || e.vk_code == VK_DELETE)
            .count();

        // 連続するキー押下ペアで、先頭がBS/Del かつ間隔>=2sのものを数える
        let del_followed_by_pause = press_ts.windows(2).zip(press_events.windows(2))
            .filter(|(ts_win, ev_win)| {
                let is_del = ev_win[0].vk_code == VK_BACK || ev_win[0].vk_code == VK_DELETE;
                let long_pause = ts_win[1].saturating_sub(ts_win[0]) >= 2000;
                is_del && long_pause
            })
            .count();

        let f6 = if del_count > 0 {
            del_followed_by_pause as f64 / del_count as f64
        } else {
            0.0
        };

        Features {
            f1_flight_time_median: f1,
            f2_flight_time_variance: f2,
            f3_correction_rate: f3,
            f4_burst_length: f4,
            f5_pause_count: f5,
            f6_pause_after_del_rate: f6,
        }
    }

    /// サイレンス期間（無入力）の合成観測値を生成する。
    /// silence_secs < 2.0 の場合は None を返す（ポーズとして認識しない）。
    /// HMM の無入力期間更新 (lib.rs の recv_timeout パス) で使用される。
    pub fn make_silence_observation(&self, silence_secs: f64) -> Option<Features> {
        // 2秒未満の無入力はポーズとして扱わない
        if silence_secs < 2.0 {
            return None;
        }

        // F1 に大きな値（2000ms）を使う。
        //
        // 理由: engine.update() は f1 <= 0.0 で早期リターンするため 0 は不可。
        //   calculate_flight_time_median() の実測値（例: 112ms）を使うと
        //   phi1 = phi(112, 150) = 0.0 となり Y_silence = 0.35*1.0 + 0.25*1.0 = 0.60。
        //   Y=0.60 は y_bin=3 (Flow優位ビン) に落ち、EWMA平衡点が 0.60 に固定されて
        //   沈黙中も Flow が維持され続ける（Incubation に遷移しない）。
        //
        //   f1=2000ms とすると phi1 = 1.0 → (1-phi1) = 0 → Y = 0.25*(1-phi5)。
        //   phi5 が増えるほど Y→0 となり、4-5秒の沈黙で y_bin が Incubation 領域に入る。
        //   「現在タイピングしていない = Flight Time が事実上無限大」と解釈する。
        let f1 = 2000.0_f64;

        // タイピングリズム系（F3/F4/F6）は0、F5（ポーズ回数）だけ沈黙の長さを反映。
        // F5定義: 「2秒以上の無入力回数」→ silence_secs / 2.0 で近似。
        // → Y = 0.25*(1-phi5): 沈黙が伸びるほど低下 → Incubation 領域へ引き寄せる。
        Some(Features {
            f1_flight_time_median: f1,
            f2_flight_time_variance: 0.0,
            f3_correction_rate: 0.0,
            f4_burst_length: 0.0,
            f5_pause_count: silence_secs / 2.0,
            f6_pause_after_del_rate: 0.0,
        })
    }
}
