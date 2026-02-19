pub mod analysis;
pub mod input;
pub mod sensors;

use std::collections::HashMap;
use std::thread;
use crossbeam_channel;
use tauri::{AppHandle, Manager, State};
use crate::analysis::{engine::{CognitiveState, CognitiveStateEngine}, features::FeatureExtractor};
use crate::sensors::SensorManager;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_cognitive_state(state: State<CognitiveStateEngine>) -> HashMap<String, f64> {
    let probs = state.get_current_state();
    let mut map = HashMap::new();
    // Convert enum keys to string for JSON serialization
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

/// C-1: Tauri 2.0 準拠のクリーンシャットダウン
#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // tracing サブスクライバの初期化
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // C-5: bounded チャンネル (64) を使用
    let (tx, rx) = crossbeam_channel::bounded(64);

    // Initialize Inference Engine
    let engine = CognitiveStateEngine::new();
    let engine_for_thread = engine.clone();
    let engine_for_monitor = engine.clone();

    // Start Analysis Thread
    thread::spawn(move || {
        tracing::info!("Analysis thread started");
        let mut extractor = FeatureExtractor::new(600);

        while let Ok(event) = rx.recv() {
            // C-3: IMEがONの間はバッファへの追加をスキップ
            if !engine_for_thread.get_paused() {
                extractor.process_event(event);
            }

            if event.is_press {
                // B-5: Features を使ってHMMを更新
                let features = extractor.calculate_features();
                if features.f1_flight_time_median > 0.0 {
                    engine_for_thread.update(&features);
                }
            }
        }
    });

    // Start IME Monitor Thread
    thread::spawn(move || {
        tracing::info!("IME Monitor thread started");
        let monitor = input::ime::ImeMonitor::new();
        loop {
            let active = monitor.is_candidate_window_open();
            engine_for_monitor.set_paused(active);
            thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // Start Input Hook
    input::hook::init_hook(tx);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::{WebviewWindowBuilder, WebviewUrl};

            // Create Overlay Window (Transparent, Click-through)
            #[cfg(desktop)]
            let _overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("index.html".into())
            )
            .title("GSE Overlay")
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .maximized(true)
            .build()?;

            #[cfg(desktop)]
            _overlay.set_ignore_cursor_events(true)?;

            // C-4: SensorManager を app.manage() で登録し、drop を防止
            let sensor_manager = SensorManager::new(app.handle().clone());
            sensor_manager.start_monitoring();
            app.manage(sensor_manager);

            Ok(())
        })
        .manage(engine)
        .invoke_handler(tauri::generate_handler![greet, get_cognitive_state, quit_app])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
