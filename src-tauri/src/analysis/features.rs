use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub vk_code: u32,
    pub timestamp: u64, // ms
    pub is_press: bool,
}

pub struct FeatureExtractor {
    buffer: VecDeque<InputEvent>,
    capacity: usize,
    last_release_time: Option<u64>,
    flight_times: VecDeque<u64>, // Store recent flight times for median calc
}

impl FeatureExtractor {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            last_release_time: None,
            flight_times: VecDeque::with_capacity(capacity), // Keep same size roughly
        }
    }

    pub fn process_event(&mut self, event: InputEvent) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(event);

        if event.is_press {
            if let Some(release_time) = self.last_release_time {
                if event.timestamp >= release_time {
                    let flight_time = event.timestamp - release_time;
                    // Filter outliers (e.g. > 2000ms is likely a pause, not typing rhythm)
                    if flight_time < 2000 {
                        self.add_flight_time(flight_time);
                    }
                }
            }
        } else {
            self.last_release_time = Some(event.timestamp);
        }
    }

    fn add_flight_time(&mut self, ft: u64) {
        if self.flight_times.len() >= self.capacity {
            self.flight_times.pop_front();
        }
        self.flight_times.push_back(ft);
    }

    pub fn calculate_flight_time_median(&self) -> f64 {
        if self.flight_times.is_empty() {
            return 0.0;
        }

        let mut sorted: Vec<u64> = self.flight_times.iter().cloned().collect();
        sorted.sort_unstable();

        let len = sorted.len();
        if len % 2 == 0 {
            let mid1 = sorted[len / 2 - 1];
            let mid2 = sorted[len / 2];
            (mid1 as f64 + mid2 as f64) / 2.0
        } else {
            sorted[len / 2] as f64
        }
    }
}
