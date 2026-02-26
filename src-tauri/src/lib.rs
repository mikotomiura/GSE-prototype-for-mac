pub mod analysis;
pub mod input;
pub mod logger;
pub mod sensors;

use crate::analysis::{
    engine::{CognitiveState, CognitiveStateEngine},
    features::FeatureExtractor,
};
use crate::logger::{LogEntry, SessionLogger};
use crate::sensors::SensorManager;
use crossbeam_channel::{self, RecvTimeoutError, Sender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Manager, State};

// ---------------------------------------------------------------------------
// ログ状態 (quit_app からも参照できるよう Tauri state で管理)
// ---------------------------------------------------------------------------

struct LogState {
    tx: Sender<LogEntry>,
    path: String,
}

// ---------------------------------------------------------------------------
// Tauri コマンド
// ---------------------------------------------------------------------------

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_cognitive_state(state: State<CognitiveStateEngine>) -> HashMap<String, f64> {
    let probs = state.get_current_state();
    let mut map = HashMap::new();
    for (k, v) in probs {
        let key_str = match k {
            CognitiveState::Flow => "flow",
            CognitiveState::Incubation => "incubation",
            CognitiveState::Stuck => "stuck",
        };
        map.insert(key_str.to_string(), v);
    }
    map
}

/// 現在のセッションログファイルのパスを返す (UI表示用)
#[tauri::command]
fn get_session_file(log: State<Arc<Mutex<LogState>>>) -> String {
    log.lock()
        .unwrap_or_else(|p| p.into_inner())
        .path
        .clone()
}

/// アプリ終了。終了前に以下を順に実行する:
///   1. セッションログを閉じる (LogEntry::End を送信)
///   2. behavioral_gt.py でラベリング分析を実行 (Python が PATH にある場合)
///   3. セッションフォルダを Explorer で開く
#[tauri::command]
fn quit_app(app: tauri::AppHandle, log: State<Arc<Mutex<LogState>>>) {
    // ログ終了マーカーを送信
    let session_path = {
        let guard = log.lock().unwrap_or_else(|p| p.into_inner());
        let _ = guard.tx.try_send(LogEntry::End);
        guard.path.clone()
    };

    let app_handle = app.clone();

    // バックグラウンドスレッドで分析→フォルダ表示→アプリ終了
    thread::spawn(move || {
        // ロガースレッドにファイルを閉じる時間を与える
        thread::sleep(std::time::Duration::from_millis(400));

        // behavioral_gt.py を探して実行
        match find_behavioral_gt() {
            Some(script) => {
                tracing::info!(
                    "Auto-analysis: python {:?} {}",
                    script,
                    session_path
                );
                match std::process::Command::new("python")
                    .args([script.to_str().unwrap_or(""), &session_path])
                    .spawn()
                {
                    Ok(_) => tracing::info!("behavioral_gt.py launched"),
                    Err(e) => tracing::warn!("Failed to launch behavioral_gt.py: {}", e),
                }
                // Python 分析に少し時間を与えてから Explorer を開く
                thread::sleep(std::time::Duration::from_millis(1500));
            }
            None => {
                tracing::warn!(
                    "behavioral_gt.py not found. Skipping auto-analysis. \
                     Run manually: python analysis/behavioral_gt.py {}",
                    session_path
                );
            }
        }

        // セッションフォルダを Explorer で開く
        if let Some(folder) = Path::new(&session_path).parent() {
            tracing::info!("Opening session folder: {:?}", folder);
            let _ = std::process::Command::new("explorer")
                .arg(folder)
                .spawn();
        }

        thread::sleep(std::time::Duration::from_millis(200));
        app_handle.exit(0);
    });
}

// ---------------------------------------------------------------------------
// behavioral_gt.py の場所を探す
// ---------------------------------------------------------------------------

/// 実行ファイルの上位ディレクトリを最大5段階遡って analysis/behavioral_gt.py を探す。
/// 開発時は CWD が gse-prototype-v2/GSE-Next なので ./analysis/ が見つかる。
fn find_behavioral_gt() -> Option<PathBuf> {
    // まず CWD 基準で探す
    let cwd_candidate = PathBuf::from("analysis/behavioral_gt.py");
    if cwd_candidate.exists() {
        return Some(cwd_candidate);
    }

    // 次に実行ファイルの上位ディレクトリを遡って探す
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..5 {
            let candidate = dir.join("analysis").join("behavioral_gt.py");
            if candidate.exists() {
                return Some(candidate);
            }
            if let Some(parent) = dir.parent() {
                dir = parent.to_path_buf();
            } else {
                break;
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// アプリ起動
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    // セッションロガー起動
    let log_path = logger::default_log_path();
    let log_path_str = log_path.to_string_lossy().to_string();
    tracing::info!("Session log: {}", log_path_str);

    let (_session_logger, log_tx) = SessionLogger::start(log_path);

    // LogState を Arc<Mutex> でラップして Tauri state に渡す
    let log_state = Arc::new(Mutex::new(LogState {
        tx: log_tx.clone(),
        path: log_path_str,
    }));

    // キーストローク入力チャネル
    let (tx, rx) = crossbeam_channel::bounded(64);

    // エンジン初期化
    let engine = CognitiveStateEngine::new();
    let engine_for_thread = engine.clone();
    let engine_for_monitor = engine.clone();
    let log_tx_ime_poll = log_tx.clone();
    let log_tx_analysis = log_tx;

    // 分析スレッド
    // イベント駆動 (rx.recv) の代わりに recv_timeout を使い、
    // 無入力期間中もタイマーでHMMを継続更新する。
    // これにより長時間ポーズ (Incubation/Stuck) を検出できる。
    thread::spawn(move || {
        tracing::info!("Analysis thread started");
        let mut extractor = FeatureExtractor::new(600);
        let mut last_event_time = Instant::now();

        loop {
            match rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(event) => {
                    if engine_for_thread.get_paused() {
                        continue;
                    }

                    last_event_time = Instant::now();
                    extractor.process_event(event);

                    // キーイベントをログ記録
                    let _ = log_tx_analysis.try_send(LogEntry::Key {
                        vk_code: event.vk_code,
                        timestamp: event.timestamp,
                        is_press: event.is_press,
                    });

                    if event.is_press {
                        // IMEモード（あ/A）はポーリングスレッドが100ms毎に更新するAtomicBoolを読む。
                        // キーフックとの同一ミリ秒flappingはこれで完全に排除される。
                        let ime_open = input::hook::IME_OPEN.load(Ordering::Relaxed);

                        let features = extractor.calculate_features();
                        engine_for_thread.update(&features, Some(event.vk_code), ime_open);

                        // 特徴量 + 状態確率をログ記録
                        let state_probs = engine_for_thread.get_current_state();
                        let p_flow = state_probs
                            .get(&CognitiveState::Flow)
                            .copied()
                            .unwrap_or(0.0);
                        let p_inc = state_probs
                            .get(&CognitiveState::Incubation)
                            .copied()
                            .unwrap_or(0.0);
                        let p_stuck = state_probs
                            .get(&CognitiveState::Stuck)
                            .copied()
                            .unwrap_or(0.0);

                        let _ = log_tx_analysis.try_send(LogEntry::Feat {
                            timestamp: event.timestamp,
                            f1: features.f1_flight_time_median,
                            f2: features.f2_flight_time_variance,
                            f3: features.f3_correction_rate,
                            f4: features.f4_burst_length,
                            f5: features.f5_pause_count,
                            f6: features.f6_pause_after_del_rate,
                            p_flow,
                            p_inc,
                            p_stuck,
                        });
                    }
                }

                Err(RecvTimeoutError::Timeout) => {
                    // 無入力期間の検出: サイレンス特徴量でHMMを更新する
                    if engine_for_thread.get_paused() {
                        continue;
                    }
                    let silence_secs = last_event_time.elapsed().as_secs_f64();
                    if let Some(sf) = extractor.make_silence_observation(silence_secs) {
                        // サイレンス中も現在のIMEモードを使用
                        let ime_open = input::hook::IME_OPEN.load(Ordering::Relaxed);
                        engine_for_thread.update(&sf, None, ime_open);

                        let state_probs = engine_for_thread.get_current_state();
                        let p_flow = state_probs
                            .get(&CognitiveState::Flow)
                            .copied()
                            .unwrap_or(0.0);
                        let p_inc = state_probs
                            .get(&CognitiveState::Incubation)
                            .copied()
                            .unwrap_or(0.0);
                        let p_stuck = state_probs
                            .get(&CognitiveState::Stuck)
                            .copied()
                            .unwrap_or(0.0);

                        let now_ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        let _ = log_tx_analysis.try_send(LogEntry::Feat {
                            timestamp: now_ts,
                            f1: sf.f1_flight_time_median,
                            f2: sf.f2_flight_time_variance,
                            f3: sf.f3_correction_rate,
                            f4: sf.f4_burst_length,
                            f5: sf.f5_pause_count,
                            f6: sf.f6_pause_after_del_rate,
                            p_flow,
                            p_inc,
                            p_stuck,
                        });
                    }
                }

                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    // IME モニタースレッド
    thread::spawn(move || {
        tracing::info!("IME Monitor thread started");
        let monitor = input::ime::ImeMonitor::new();
        loop {
            let active = monitor.is_candidate_window_open();
            engine_for_monitor.set_paused(active);
            if active {
                engine_for_monitor.force_flow_state();
            }
            thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // IME open ポーリングスレッド (wake-on-keystroke + VK_DBE_* fallback)
    //
    // wake チャネル (bounded 1):
    //   フックコールバックがキー押下ごとに非ブロッキング try_send → ポーリングスレッドが
    //   recv_timeout(100ms) で即起床 → 5ms 待機後に ImmGetOpenStatus または
    //   IME_STATE_DIRTY フラグで状態を確認。
    //   同一ミリ秒 flapping 不可能（LogEntry::ImeState はポーリングスレッドのタイムスタンプ）。
    //   アイドル時最大ラグ 100ms → アクティブ打鍵時最大ラグ ~5ms。
    let (poll_wake_tx, poll_wake_rx) = crossbeam_channel::bounded::<()>(1);
    input::hook::set_poll_wake_sender(poll_wake_tx);
    input::ime::spawn_ime_open_polling_thread(log_tx_ime_poll, poll_wake_rx);

    // キーボードフック開始 (set_poll_wake_sender の後に呼ぶこと)
    input::hook::init_hook(tx);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let sensor_manager = SensorManager::new(app.handle().clone());
            app.manage(sensor_manager);
            let sensor_state: State<SensorManager<tauri::Wry>> = app.state();
            sensor_state.start_monitoring();
            Ok(())
        })
        .manage(engine)
        .manage(log_state)
        .invoke_handler(tauri::generate_handler![
            greet,
            get_cognitive_state,
            quit_app,
            get_session_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
