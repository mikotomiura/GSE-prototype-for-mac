pub mod analysis;
pub mod input;
pub mod logger;
pub mod sensors;
pub mod wall_server;

use crate::analysis::{
    engine::{CognitiveState, CognitiveStateEngine},
    features::FeatureExtractor,
};
use crate::logger::{LogEntry, SessionLogger};
use crate::sensors::SensorManager;
use crossbeam_channel::{self, RecvTimeoutError, Sender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Manager, State};

// ---------------------------------------------------------------------------
// リセットシグナル (start_session → 分析スレッド)
// ---------------------------------------------------------------------------

struct ResetSignal(Arc<AtomicBool>);

// ---------------------------------------------------------------------------
// セッション開始フラグ (start_session で true にセット)
// false の間、分析スレッドはイベントをドレインするが処理しない
// ---------------------------------------------------------------------------

struct SessionActive(Arc<AtomicBool>);

// ---------------------------------------------------------------------------
// ログ状態 (quit_app からも参照できるよう Tauri state で管理)
// ---------------------------------------------------------------------------

struct LogState {
    tx: Sender<LogEntry>,
    path: String,
    shutdown_rx: crossbeam_channel::Receiver<()>,
}

// ---------------------------------------------------------------------------
// Tauri コマンド
// ---------------------------------------------------------------------------

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

/// 最終キーストロークからの経過時間（ミリ秒）を返す。
/// 打鍵がまだ無い場合は 0 を返す。
#[tauri::command]
fn get_keyboard_idle_ms() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = crate::input::hook::LAST_KEYSTROKE_TIMESTAMP.load(Ordering::Relaxed);
    if last == 0 { 0 } else { now.saturating_sub(last) }
}

/// 最終キーストロークの UNIX エポックミリ秒タイムスタンプを返す。
/// 打鍵がまだ無い場合は 0 を返す。
/// フロントエンドで Wall 発動後の打鍵かどうかを判定するために使用。
#[tauri::command]
fn get_last_keypress_timestamp() -> u64 {
    crate::input::hook::LAST_KEYSTROKE_TIMESTAMP.load(Ordering::Relaxed)
}

/// キーボードフック（Input Monitoring）の状態を返す。
/// Windows: 常に true（権限不要）。
/// macOS: CGEventTap が正常にインストールされている場合のみ true。
///        false の場合はフロントエンドで権限バナーを表示する。
#[tauri::command]
fn get_hook_status() -> bool {
    #[cfg(target_os = "macos")]
    return crate::input::hook::hook_macos::HOOK_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(target_os = "macos"))]
    return true;
}

// ---------------------------------------------------------------------------
// Wall Server 状態
// ---------------------------------------------------------------------------

type WallServerState = Arc<Mutex<Option<wall_server::WallServer>>>;

/// Wall unlock サーバーを起動し、QR コード SVG と URL を返す。
/// 既に起動中の場合は既存の情報を返す。
#[tauri::command]
fn start_wall_server(
    app: tauri::AppHandle,
    wall: State<WallServerState>,
) -> Result<wall_server::WallServerInfo, String> {
    let mut guard = wall.lock().map_err(|e| e.to_string())?;
    if let Some(ref existing) = *guard {
        return Ok(existing.info().clone());
    }
    let (server, info) = wall_server::WallServer::start(app)?;
    *guard = Some(server);
    Ok(info)
}

/// Wall unlock サーバーを停止する。
#[tauri::command]
fn stop_wall_server(wall: State<WallServerState>) {
    let mut guard = wall.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(server) = guard.take() {
        server.stop();
    }
}

/// セッションを開始する。
/// 1. CognitiveStateEngine をリセット (HMM初期化)
/// 2. ResetSignal を true にセット (分析スレッドへの信号)
/// 3. LogEntry::SessionStart をログに記録 (境界マーカー)
#[tauri::command]
fn start_session(
    engine: State<CognitiveStateEngine>,
    reset: State<ResetSignal>,
    active: State<SessionActive>,
    log: State<Arc<Mutex<LogState>>>,
) {
    // 1. HMM を初期値にリセット
    engine.reset();

    // 2. 分析スレッドへリセットシグナルを送信 (MUST be before session_active)
    //    Release ordering: reset=true が active=true より先にグローバルに可視となる。
    //    分析スレッドは active=true を Acquire で検知した時点で、reset=true も必ず可視。
    //    旧順序 (active→reset) では、分析スレッドが active=true を見た瞬間に
    //    reset=true が未反映で、前セッションの長い沈黙が初回HMM更新に混入していた。
    reset.0.store(true, Ordering::Release);

    // 3. セッションをアクティブにする（分析スレッドの処理を開始）
    active.0.store(true, Ordering::Release);

    // 4. セッション開始マーカーをログに記録
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let guard = log.lock().unwrap_or_else(|p| p.into_inner());
    let _ = guard.tx.try_send(LogEntry::SessionStart { timestamp });

    tracing::info!("Session started (engine reset, signal sent)");
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
fn quit_app(app: tauri::AppHandle, log: State<Arc<Mutex<LogState>>>, wall: State<WallServerState>) {
    // Wall server をクリーンアップ
    {
        let mut guard = wall.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(server) = guard.take() {
            server.stop();
        }
    }

    // ログ終了マーカーを送信し、シャットダウン完了チャネルを取得
    let (session_path, shutdown_rx) = {
        let guard = log.lock().unwrap_or_else(|p| p.into_inner());
        let _ = guard.tx.try_send(LogEntry::End);
        (guard.path.clone(), guard.shutdown_rx.clone())
    };

    let app_handle = app.clone();

    // バックグラウンドスレッドで分析→フォルダ表示→アプリ終了
    thread::spawn(move || {
        // ロガースレッドの終了を確実に待機（最大2秒）
        let _ = shutdown_rx.recv_timeout(std::time::Duration::from_secs(2));

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

        // セッションフォルダをFinderで開く
        if let Some(folder) = Path::new(&session_path).parent() {
            tracing::info!("Opening session folder: {:?}", folder);
            let _ = std::process::Command::new("open").arg(folder).spawn();
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

    let (_session_logger, log_tx, log_shutdown_rx) = SessionLogger::start(log_path);

    // LogState を Arc<Mutex> でラップして Tauri state に渡す
    let log_state = Arc::new(Mutex::new(LogState {
        tx: log_tx.clone(),
        path: log_path_str,
        shutdown_rx: log_shutdown_rx,
    }));

    // キーストローク入力チャネル
    let (tx, rx) = crossbeam_channel::bounded(64);

    // エンジン初期化
    let engine = CognitiveStateEngine::new();
    let engine_for_thread = engine.clone();
    let engine_for_monitor = engine.clone();
    let log_tx_ime_poll = log_tx.clone();
    let log_tx_analysis = log_tx;

    // リセットシグナル (start_session → 分析スレッド)
    let reset_signal = Arc::new(AtomicBool::new(false));
    let reset_signal_for_thread = reset_signal.clone();
    let reset_state = ResetSignal(reset_signal);

    // セッション開始フラグ (false = スタート画面中、true = セッション中)
    let session_active = Arc::new(AtomicBool::new(false));
    let session_active_for_thread = session_active.clone();
    let session_active_state = SessionActive(session_active);

    // 分析スレッド
    //
    // 【アーキテクチャ v2 (Fix 1/2/4)】
    // 旧: recv_timeout(1000ms) でイベント駆動 + タイムアウト時にサイレンス処理。
    //   問題: 1Hzゲートがキーpressイベントに依存し、最大9.6秒のfeatギャップが発生。
    //         タイムアウト時に make_silence_observation() が30秒ウィンドウの実データを無視。
    //         1打鍵で silence_secs が即座にリセットされ合成摩擦値が消失。
    //
    // 新: 動的 recv_timeout(until_gate) + 独立1Hzゲート。
    //   - イベント処理と1Hz推論を完全分離。タイムアウトは次の1Hzマークまでの残り時間。
    //   - 1Hzゲートでは calculate_features() を優先し、f1>0（実データ有）なら
    //     engine.update() を使用。f1=0（バッファ空）の場合のみ silence obs にフォールバック。
    //   - 深い沈黙（>10秒）からの復帰は3回以上の連続打鍵を要求（silence-break protection）。
    thread::spawn(move || {
        tracing::info!("Analysis thread started");
        let mut extractor = FeatureExtractor::new(600);
        let mut last_event_time = Instant::now();
        // 次回エンジン更新予定時刻。絶対時刻ベースで += 1s インクリメントし、
        // イベント処理遅延によるドリフトを防ぐ。
        let mut next_engine_update = Instant::now();
        // Fix 2: 深い沈黙からの復帰に3回以上の連続打鍵を要求するカウンター。
        // 1打鍵で last_event_time がリセットされ、合成摩擦値 (f3, f6) が
        // 即座に消失する問題を防止する。
        let mut presses_since_silence: u32 = 0;

        loop {
            // リセットシグナルチェック: start_session から送信される
            if reset_signal_for_thread.swap(false, Ordering::AcqRel) {
                tracing::info!("Analysis thread: reset signal received");
                extractor.reset();
                last_event_time = Instant::now();
                next_engine_update = Instant::now();
                presses_since_silence = 0;
            }

            // Fix 4: 動的タイムアウト — 次の1Hzマークまたはイベント到着で起床。
            // 旧 recv_timeout(1000ms) ではキーイベントのタイミングに1Hzゲートが依存し、
            // 最大数秒のfeatギャップが発生していた。動的タイムアウトにより、
            // イベントの有無に関わらず正確な1Hz間隔で推論が実行される。
            let until_gate = next_engine_update.saturating_duration_since(Instant::now());

            match rx.recv_timeout(until_gate) {
                Ok(event) => {
                    // セッション未開始 — イベントをドレイン（チャネル詰まり防止）するが処理しない
                    if !session_active_for_thread.load(Ordering::Acquire) {
                        continue;
                    }
                    // リセット保留中 — リセット前の古いイベントをスキップ
                    // (次のループ先頭で reset_signal が処理される)
                    if reset_signal_for_thread.load(Ordering::Acquire) {
                        continue;
                    }

                    if engine_for_thread.get_paused() {
                        continue;
                    }

                    extractor.process_event(event);

                    // キーイベントをログ記録（全打鍵を記録、推論頻度とは独立）
                    let _ = log_tx_analysis.try_send(LogEntry::Key {
                        vk_code: event.vk_code,
                        timestamp: event.timestamp,
                        is_press: event.is_press,
                    });

                    if event.is_press {
                        // 全打鍵でBackspaceストリークをカウント（1Hz gate の外側）。
                        // engine.update() は1Hzだがストリーク検知は全打鍵で正確にカウントする。
                        // OS別 Backspace 判定:
                        //   Windows: VK_BACK = 0x08
                        //   macOS: macos_vk_to_vk() が 0x33→0x08 に変換済み
                        // engine 層にはOS固有キーコードを渡さず、bool で抽象化する。
                        let is_backspace = event.vk_code == 0x08;
                        engine_for_thread.register_keystroke(is_backspace);

                        // Fix 2: silence-break protection.
                        // 深い沈黙（>10秒）中は、単発キーで last_event_time をリセットせず、
                        // 3回以上の連続打鍵で初めてリセットする。
                        // これにより、Stuck状態の合成摩擦値 (f3, f6) が1打鍵で消失するのを防ぐ。
                        if last_event_time.elapsed() > Duration::from_secs(10) {
                            presses_since_silence += 1;
                            if presses_since_silence >= 3 {
                                last_event_time = Instant::now();
                                presses_since_silence = 0;
                            }
                        } else {
                            last_event_time = Instant::now();
                            presses_since_silence = 0;
                        }
                    } else if last_event_time.elapsed() <= Duration::from_secs(10) {
                        // Release events: 深い沈黙中でなければタイマーを更新
                        last_event_time = Instant::now();
                    }
                }

                Err(RecvTimeoutError::Timeout) => {
                    // タイムアウト: 1Hzゲートへ直接フォールスルー
                }

                Err(RecvTimeoutError::Disconnected) => break,
            }

            // ── 1Hz ゲート (Fix 4: イベント到着と独立) ──────────────────────
            // セッション・ポーズガード
            if !session_active_for_thread.load(Ordering::Acquire) {
                continue;
            }
            if reset_signal_for_thread.load(Ordering::Acquire) {
                continue;
            }
            if engine_for_thread.get_paused() {
                continue;
            }

            let now = Instant::now();
            if now >= next_engine_update {
                // 絶対時刻ベースで1Hzを維持（ドリフト防止）
                next_engine_update += Duration::from_secs(1);
                // スレッドが長時間ブロックされた場合のキャッチアップ
                if next_engine_update <= now {
                    next_engine_update = now + Duration::from_secs(1);
                }

                // IMEモード（あ/A）はポーリングスレッドが100ms毎に更新するAtomicBoolを読む。
                let ime_open = input::hook::IME_OPEN.load(Ordering::Relaxed);
                let silence_secs = last_event_time.elapsed().as_secs_f64();

                // Fix 1: バッファの実データを優先し、空の場合のみサイレンス観測にフォールバック。
                // 直近30秒に有効な打鍵データ (f1 > 0) がある場合は実特徴量で HMM を更新する。
                // これにより、短いポーズ（2-3秒）で30秒ウィンドウのデータが無視される問題を解消。
                let mut features = extractor.calculate_features();

                // Fix 5: 現在進行形の沈黙をF5に加算する。
                // calculate_features() は30秒ウィンドウ内の「完了済みギャップ」しかカウントしない。
                // 例: 12秒間手が止まっていてもF5=1.0にしかならず、進行中の行き詰まりが
                // 摩擦として評価されない。silence_secs / 2.0 を加算し、
                // 現在のギャップに相当するポーズ回数をF5に反映する。
                if silence_secs >= 2.0 && features.f1_flight_time_median > 0.0 {
                    features.f5_pause_count += silence_secs / 2.0;
                }

                // Fix 6: 深い沈黙時に F3/F6 の摩擦フロアを動的に適用する。
                // 30秒ウィンドウに過去の順調なタイピングデータが残っていると、
                // F3(修正率)とF6(削除後停止率)が低いままで Friction 軸が上がらず
                // Stuck 領域（x_bin≥3）に到達できない。
                // 沈黙時間に応じて段階的にフロア値を引き上げ、
                // 「現在手が止まっている」事実を Friction に反映する。
                if silence_secs >= 8.0 && features.f1_flight_time_median > 0.0 {
                    let f6_floor = ((silence_secs - 8.0) / 25.0).clamp(0.0, 0.50);
                    let f3_floor = ((silence_secs - 10.0) / 30.0).clamp(0.0, 0.40);
                    features.f6_pause_after_del_rate = features.f6_pause_after_del_rate.max(f6_floor);
                    features.f3_correction_rate = features.f3_correction_rate.max(f3_floor);
                }

                let (log_features, diag) = if features.f1_flight_time_median > 0.0
                    && silence_secs < 5.0
                {
                    // 実打鍵データがウィンドウ内にあり、直近5秒以内に打鍵あり
                    // — 通常更新 (α=0.3 EWMA)
                    let d = engine_for_thread.update(&features, ime_open);
                    (features, d)
                } else if features.f1_flight_time_median > 0.0 {
                    // 30秒ウィンドウに実データはあるが、5秒以上手が止まっている。
                    // 沈黙の深さに応じた動的α (0.15〜0.25) で更新する。
                    let d = engine_for_thread.update_silence(&features, ime_open, silence_secs);
                    (features, d)
                } else if let Some(sf) = extractor.make_silence_observation(silence_secs) {
                    // バッファ空 — サイレンス観測にフォールバック
                    let d = engine_for_thread.update_silence(&sf, ime_open, silence_secs);
                    (sf, d)
                } else {
                    // silence < 2.0s かつバッファ空 — スキップ
                    continue;
                };

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

                let now_ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                // diagnostics が None (paused 中の early return) の場合のデフォルト値
                let (raw_x, raw_y, ewma_x, ewma_y, obs, alpha) = match diag {
                    Some(d) => (d.raw_x, d.raw_y, d.ewma_x, d.ewma_y, d.obs, d.alpha),
                    None => (0.0, 0.0, 0.0, 0.0, 0, 0.0),
                };

                let _ = log_tx_analysis.try_send(LogEntry::Feat {
                    timestamp: now_ts,
                    f1: log_features.f1_flight_time_median,
                    f3: log_features.f3_correction_rate,
                    f4: log_features.f4_burst_length,
                    f5: log_features.f5_pause_count,
                    f6: log_features.f6_pause_after_del_rate,
                    p_flow,
                    p_inc,
                    p_stuck,
                    raw_x,
                    raw_y,
                    ewma_x,
                    ewma_y,
                    obs,
                    alpha,
                });
            }
        }
    });

    // IME モニタースレッド
    // CGWindowListCopyWindowInfo で日本語IMEの候補ウィンドウを100msポーリング検出。
    // 候補ウィンドウ表示中はHMMを一時停止し、ナビゲーションキー操作が特徴量を汚染するのを防ぐ。
    thread::spawn(move || {
        tracing::info!("IME Monitor thread started");
        let monitor = input::ime::ImeMonitor::new();
        let mut last_active = false;
        // IME_ACTIVE が true になった時刻を記録（8秒フェイルセーフ用）
        let mut active_since: Option<Instant> = None;

        loop {
            let active = monitor.is_candidate_window_open();

            // 8秒フェイルセーフ: IME_ACTIVE が8秒以上継続した場合、
            // OSイベントの取りこぼし（stale）とみなしてリセットする。
            let active = if active {
                match active_since {
                    Some(since) if since.elapsed() >= Duration::from_secs(8) => {
                        tracing::warn!(
                            "IME candidate window stale (>8s) — forcing reset"
                        );
                        active_since = None;
                        false // stale として false にリセット
                    }
                    Some(_) => true, // まだ8秒未満
                    None => {
                        active_since = Some(Instant::now());
                        true
                    }
                }
            } else {
                active_since = None;
                false
            };

            if active != last_active {
                tracing::debug!(
                    "IME candidate window: {}",
                    if active { "OPEN (pausing HMM)" } else { "CLOSED (resuming HMM)" }
                );
                last_active = active;
            }

            input::hook::IME_ACTIVE.store(active, Ordering::Release);
            // IME候補ウィンドウ表示中はHMM推論を一時停止する。
            // 確率はリセットせず直前の状態を維持する。
            // 候補選択中の認知状態（Incubation等）を破壊しない。
            engine_for_monitor.set_paused(active);
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

    // Wall unlock サーバー状態
    let wall_state: WallServerState = Arc::new(Mutex::new(None));

    // macOS ファイアウォールのダイアログをオーバーレイ表示前にトリガーするため、
    // 起動時にダミーの TCP リスナーをバインド → ローカル接続 → 即クローズ
    std::thread::spawn(|| {
        if let Ok(listener) = std::net::TcpListener::bind("0.0.0.0:0") {
            if let Ok(addr) = listener.local_addr() {
                let _ = std::net::TcpStream::connect(addr);
            }
        }
    });

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
        .manage(wall_state)
        .manage(reset_state)
        .manage(session_active_state)
        .invoke_handler(tauri::generate_handler![
            get_cognitive_state,
            get_keyboard_idle_ms,
            get_last_keypress_timestamp,
            quit_app,
            get_session_file,
            get_hook_status,
            start_wall_server,
            stop_wall_server,
            start_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
