// macOS sensor implementation.
// Re-exports SensorManager from the platform-specific implementation.

#[path = "sensors_macos.rs"]
mod platform;

pub use platform::SensorManager;
