Physical Anchor Specification (Surface Pro 8)
Objective
Detect user movement (walking) to unlock the screen ("The Wall").

Requirements
1.Accelerometer (WinRT):

Use windows::Devices::Sensors::Accelerometer.

Set ReportInterval to approx 16ms (60Hz) for FFT analysis.

Detect walking rhythm (1.5Hz - 2.0Hz signal).

2.Geolocation (WinRT):

Use windows::Devices::Geolocation::Geolocator.

Calculate displacement (distance from lock point).

Threshold: > 100m movement.

3.Implementation Detail:

These APIs are async. Use Tokio to handle WinRT futures bridge if necessary.

Handle cases where sensors are unavailable (desktop mode fallback).

Let's think step by step.