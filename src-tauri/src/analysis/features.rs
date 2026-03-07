use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub vk_code: u32,
    pub timestamp: u64, // ms
    pub is_press: bool,
    /// macOS: CGEventTap の kCGKeyboardEventAutorepeat フラグ。
    /// キーリピートは press のみ連射され release が出ないため、
    /// flight time 計算から除外する必要がある。
    pub is_repeat: bool,
    /// Backspace または Delete キーかどうか。
    /// OS 固有のキーコード判定はフック層で行い、共通コアはこのフラグのみを参照する。
    /// Windows: VK_BACK(0x08) || VK_DELETE(0x2E)
    /// macOS: kVK_Delete(0x33) || kVK_ForwardDelete(0x75)
    pub is_backspace: bool,
}

/// B-1: 5特徴量を格納する構造体 (F1,F3,F4,F5,F6; F2は未使用のため削除)
#[derive(Debug, Clone)]
pub struct Features {
    /// F1: Flight Time 中央値 (ms)
    pub f1_flight_time_median: f64,
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

/// Flight time 計算に含めるべき「タイピング動作」キーかどうかを判定する。
/// 矢印キー・IMEモードキー等はタイピングリズムとは無関係なので除外する。
pub fn is_typing_key(vk: u32) -> bool {
    matches!(vk,
        // Letters A-Z
        0x41..=0x5A
        // Digits 0-9
        | 0x30..=0x39
        // Common punctuation/symbols
        | 0xBA..=0xC0  // ;=,-./`
        | 0xDB..=0xDF  // [\]'^
        // Editing keys (affect typing rhythm)
        | 0x08  // VK_BACK
        | 0x0D  // VK_RETURN
        | 0x20  // VK_SPACE
        | 0x09  // VK_TAB
        | 0x2E  // VK_DELETE
    )
}

pub struct FeatureExtractor {
    buffer: VecDeque<InputEvent>,
    capacity: usize,
}

impl FeatureExtractor {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// バッファをクリアする。
    /// セッション開始時に呼び出され、前回セッションのデータを破棄する。
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    pub fn process_event(&mut self, event: InputEvent) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(event);
    }

    /// B-1: 直近30秒のバッファから5特徴量を算出する
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

        // --- ウィンドウ内のフライトタイムを計算 (直近30秒) ---
        // キーリピートと非タイピングキーを除外して flight time を算出する。
        let mut window_fts: Vec<f64> = Vec::new();
        let mut last_release: Option<u64> = None;
        for event in &events {
            if event.is_repeat || !is_typing_key(event.vk_code) {
                continue;
            }
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

        // --- F1: Flight Time 中央値 (直近30秒ウィンドウ) ---
        let f1 = if window_fts.is_empty() {
            0.0
        } else {
            window_fts.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let len = window_fts.len();
            if len % 2 == 0 {
                (window_fts[len / 2 - 1] + window_fts[len / 2]) / 2.0
            } else {
                window_fts[len / 2]
            }
        };

        // --- キー押下イベントのみ抽出 ---
        let press_events: Vec<&&InputEvent> = events.iter()
            .filter(|e| e.is_press)
            .collect();

        let total_keys = press_events.len();

        // --- F3: 修正率 = (BS + Del) / 全キー押下数 ---
        let correction_keys = press_events.iter()
            .filter(|e| e.is_backspace)
            .count();

        let f3 = if total_keys > 0 {
            correction_keys as f64 / total_keys as f64
        } else {
            0.0
        };

        // --- F4: バースト長 = 連続FT<200ms のチャンクの平均文字数 ---
        // キーリピートと非タイピングキーを除外してバースト計算する。
        let mut burst_lengths: Vec<usize> = Vec::new();
        let mut current_burst: usize = 0;
        let mut last_rel_for_burst: Option<u64> = None;

        for event in &events {
            if event.is_repeat || !is_typing_key(event.vk_code) {
                continue;
            }
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
            .filter(|e| e.is_backspace)
            .count();

        // 連続するキー押下ペアで、先頭がBS/Del かつ間隔>=2sのものを数える
        let del_followed_by_pause = press_ts.windows(2).zip(press_events.windows(2))
            .filter(|(ts_win, ev_win)| {
                let long_pause = ts_win[1].saturating_sub(ts_win[0]) >= 2000;
                ev_win[0].is_backspace && long_pause
            })
            .count();

        let f6 = if del_count > 0 {
            del_followed_by_pause as f64 / del_count as f64
        } else {
            0.0
        };

        Features {
            f1_flight_time_median: f1,
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
        //   実測 FT 中央値（例: 112ms）を使うと
        //   phi1 = phi(112, 150) = 0.0 となり Y_silence = 0.35*1.0 + 0.25*1.0 = 0.60。
        //   Y=0.60 は y_bin=3 (Flow優位ビン) に落ち、EWMA平衡点が 0.60 に固定されて
        //   沈黙中も Flow が維持され続ける（Incubation に遷移しない）。
        //
        //   f1=2000ms とすると phi1 = 1.0 → (1-phi1) = 0 → Y = 0.25*(1-phi5)。
        //   phi5 が増えるほど Y→0 となり、4-5秒の沈黙で y_bin が Incubation 領域に入る。
        //   「現在タイピングしていない = Flight Time が事実上無限大」と解釈する。
        let f1 = 2000.0_f64;

        // F5（ポーズ回数）: 沈黙の長さを反映。
        // F5定義: 「2秒以上の無入力回数」→ silence_secs / 2.0 で近似。
        // → Y = 0.25*(1-phi5): 沈黙が伸びるほど低下 → Incubation 領域へ引き寄せる。

        // Synthetic Friction（合成摩擦値）:
        // F5 だけでは Friction 軸 X が最大 0.45 に留まり x_bin=2 止まり。
        // Incubation 領域（x=0..2）から Stuck 領域（x=3,4）へ遷移できない。
        // 長期沈黙時に F6/F3 を漸増させ、Friction を引き上げて Stuck 判定を可能にする。
        //
        // 旧オンセット: f6 @ 20s, f3 @ 30s — Stuck検出が遅く、リセット後の再建も遅い。
        // 新オンセット: f6 @ 8s, f3 @ 15s — より早期に摩擦を蓄積し、迅速にStuck領域へ到達。
        let mut f6_synthetic = ((silence_secs - 8.0) / 50.0).clamp(0.0, 0.50);
        let mut f3_synthetic = ((silence_secs - 15.0) / 80.0).clamp(0.0, 0.40);

        // 摩擦フロア: 10秒以上の沈黙では最低摩擦値を維持する。
        // 観測ビンが純粋な Incubation 領域へドリフトし、沈黙だけで
        // Stuck から「自動回復」してしまう現象を防止する。
        // フロア値は β_writing (f3:0.18, f6:0.12) を上回るよう設定し、
        // IME入力モードでも phi > 0 となるようにする。
        if silence_secs >= 10.0 {
            f6_synthetic = f6_synthetic.max(0.15);
            f3_synthetic = f3_synthetic.max(0.20);
        }

        Some(Features {
            f1_flight_time_median: f1,
            f3_correction_rate: f3_synthetic,
            f4_burst_length: 0.0,
            f5_pause_count: silence_secs / 2.0,
            f6_pause_after_del_rate: f6_synthetic,
        })
    }
}
