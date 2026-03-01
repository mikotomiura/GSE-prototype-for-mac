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
// Impact: Native accelerometer events are not emitted on macOS.
// Wall unlock uses smartphone QR code + DeviceMotion instead (see wall_server.rs).
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
        tracing::info!(
            "SensorManager: native accelerometer not available on macOS. \
             Wall unlock uses smartphone QR code motion detection instead."
        );
        // No sensor threads spawned on macOS.
    }
}
