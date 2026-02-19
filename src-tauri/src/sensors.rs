use tauri::{AppHandle, Emitter, Runtime};
use windows::{
    Devices::{
        Geolocation::Geolocator,
        Sensors::Accelerometer,
    },
    Foundation::TypedEventHandler,
};
use std::thread;
use std::time::Duration;

pub struct SensorManager<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> SensorManager<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    pub fn start_monitoring(&self) {
        let app_handle = self.app.clone();

        // Accelerometer Thread
        thread::spawn(move || {
            // Initialize WinRT/COM for this thread
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            
            // Attempt to get default accelerometer
            let accel_result = Accelerometer::GetDefault();
            match accel_result {
                Ok(accel) => {
                    tracing::info!("Accelerometer detected.");
                    // Set Report Interval (approx 60Hz -> 16ms)
                    let min_interval = accel.MinimumReportInterval().unwrap_or(16);
                    let target_interval = std::cmp::max(min_interval, 16);
                    let _ = accel.SetReportInterval(target_interval);

                    // Subscribe to ReadingChanged
                    // We need to keep the event token or the object alive? 
                    // In Rust windows crate, the handler is kept alive by the object? 
                    // Actually, we usually need to keep the object alive.
                    
                    let app_clone = app_handle.clone();
                    let handler = TypedEventHandler::new(move |_, args: &Option<windows::Devices::Sensors::AccelerometerReadingChangedEventArgs>| {
                        if let Some(args) = args {
                            if let Ok(reading) = args.Reading() {
                                if let Ok(vals) = reading.AccelerationX() { // Just checking one for now or all
                                    let x = reading.AccelerationX().unwrap_or(0.0);
                                    let y = reading.AccelerationY().unwrap_or(0.0);
                                    let z = reading.AccelerationZ().unwrap_or(0.0);
                                    
                                    // Simple magnitude check for "Move"
                                    let magnitude = (x*x + y*y + z*z).sqrt();
                                    
                                    // 1.0 is gravity. Significant movement > 1.2 or < 0.8
                                    if (magnitude - 1.0).abs() > 0.2 {
                                        let _ = app_clone.emit("sensor-accelerometer", "move");
                                    }
                                }
                            }
                        }
                        Ok(())
                    });

                    if let Ok(_token) = accel.ReadingChanged(&handler) {
                        // Keep thread alive to hold the reference? 
                        // The `accel` object needs to be kept alive.
                        loop {
                            thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
                _ => {
                    tracing::warn!("No Accelerometer found (Fallback mode).");
                }
            }
        });

        // Geolocator Thread
        let app_handle_geo = self.app.clone();
        thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }

            let geolocator = Geolocator::new();
            if let Ok(geo) = geolocator {
                 // Request Access (might block or return async?)
                 // RequestAccessAsync returns IAsyncOperation. We should wait for it.
                 // For simplicity in this non-async thread, we might skip explicit request if getting position works?
                 // But usually we need it.
                 
                 // Using polling for simplicity if async await is hard in vanilla thread.
                 // Or we can just try to subscribe.
                 
                let app_clone = app_handle_geo.clone();
                let handler = TypedEventHandler::new(move |_, args: &Option<windows::Devices::Geolocation::PositionChangedEventArgs>| {
                    if let Some(args) = args {
                        if let Ok(pos) = args.Position() {
                             if let Ok(coord) = pos.Coordinate() {
                                 if let Ok(point) = coord.Point() {
                                     if let Ok(geo_pos) = point.Position() {
                                         // Emit minimal data
                                         let payload = format!("{{ \"lat\": {}, \"lng\": {} }}", geo_pos.Latitude, geo_pos.Longitude);
                                         let _ = app_clone.emit("sensor-geolocation", payload);
                                     }
                                 }
                             }
                        }
                    }
                    Ok(())
                });

                 if let Ok(_token) = geo.PositionChanged(&handler) {
                     loop {
                         thread::sleep(Duration::from_secs(5));
                     }
                 }
            } else {
                tracing::warn!("Geolocator not available.");
            }
        });
    }
}
