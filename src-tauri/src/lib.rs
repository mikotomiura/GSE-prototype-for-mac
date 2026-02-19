pub mod analysis;
pub mod input;
pub mod sensors;

use std::collections::HashMap;
use std::thread;
use crossbeam_channel;
use tauri::State;
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

#[tauri::command]
fn quit_app() {
    std::process::exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize Sensing Layer
    let (tx, rx) = crossbeam_channel::unbounded();

    // Initialize Inference Engine
    let engine = CognitiveStateEngine::new();
    let engine_for_thread = engine.clone();
    let engine_for_monitor = engine.clone(); // Clone for IME monitor

    // Start Analysis Thread
    thread::spawn(move || {
        println!("Analysis thread started");
        let mut extractor = FeatureExtractor::new(600);
        
        while let Ok(event) = rx.recv() {
            extractor.process_event(event);
            
            if event.is_press {
                let ft = extractor.calculate_flight_time_median();
                if ft > 0.0 {
                    // Update HMM
                    engine_for_thread.update(ft);
                    
                    // Optional logging
                    println!("FT: {:.2}, State: {:?}", ft, engine_for_thread.get_current_state());
                }
            }
        }
    });

    // Start IME Monitor Thread
    thread::spawn(move || {
        println!("IME Monitor thread started");
        let monitor = input::ime::ImeMonitor::new();
        loop {
            let active = monitor.is_candidate_window_open();
            // println!("IME Active: {}", active);
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

            // Initialize Sensors (Phase 4)
            let sensor_manager = SensorManager::new(app.handle().clone());
            sensor_manager.start_monitoring();

            Ok(())
        })
        .manage(engine) // Manage the engine state for commands
        .invoke_handler(tauri::generate_handler![greet, get_cognitive_state, quit_app])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
