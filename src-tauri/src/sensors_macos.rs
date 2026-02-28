// macOS sensor integration stub.
//
// Full CoreMotion (CMMotionManager) accelerometer integration requires
// implementing objc2::Encode for CMAcceleration and CoreMotion framework linking,
// which adds significant complexity for a supplementary feature.
//
// Current status:
// - Accelerometer: stub (not available) — MacBook HW support can be added later
// - Geolocation:   not implemented (unused in HMM features F1-F6)
//
// Impact: The "sensor-accelerometer" Tauri event is never emitted on macOS.
// The App.tsx wall-unlock feature (shake to escape Stuck state) will not trigger.
// All core Flow/Incubation/Stuck estimation via keystroke dynamics is unaffected.

use tauri::{AppHandle, Runtime};

pub struct SensorManager<R: Runtime> {
    #[allow(dead_code)]
    app: AppHandle<R>,
}

impl<R: Runtime> SensorManager<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    pub fn start_monitoring(&self) {
        tracing::warn!(
            "SensorManager: accelerometer monitoring not implemented on macOS. \
             Keystroke dynamics features (F1-F6) are unaffected. \
             The shake-to-unlock Stuck wall will not trigger."
        );
        // No sensor threads spawned on macOS.
    }
}
