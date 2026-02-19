use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CognitiveState {
    Flow,
    Incubation,
    Stuck,
}

#[derive(Clone)]
pub struct CognitiveStateEngine {
    // Manual HMM parameters
    transitions: Arc<[f64; 9]>,
    emissions: Arc<[f64; 33]>,
    
    current_state_probs: Arc<Mutex<[f64; 3]>>, 
    pub is_paused: Arc<Mutex<bool>>, // New field
}

impl CognitiveStateEngine {
    pub fn new() -> Self {
        // ... (existing params)
        // Transitions (A) - Updated based on gpt-advice.md
        // Flow -> Flow (0.7), Incubation (0.25), Stuck (0.05)
        // Incubation -> Flow (0.2), Incubation (0.6), Stuck (0.2)
        // Stuck -> Flow (0.1), Incubation (0.2), Stuck (0.7)
        let transitions = [
            0.7, 0.25, 0.05,
            0.2, 0.6, 0.2,
            0.1, 0.2, 0.7,
        ];

        // Emissions (B) 3x11
        // Flow (State 0): Bias to 0-3
        // Inc (State 1): Bias to 3-7
        // Stuck (State 2): Bias to 7-10
        #[rustfmt::skip]
        let emissions = [
            // Flow
            0.3, 0.3, 0.2, 0.1, 0.05, 0.02, 0.01, 0.01, 0.005, 0.005, 0.0,
            // Incubation
            0.01, 0.02, 0.05, 0.1, 0.2, 0.2, 0.2, 0.1, 0.05, 0.04, 0.03,
            // Stuck
            0.0, 0.005, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.3, 0.3, 0.01,
        ];

        // Initial Priors
        let initial_probs = [0.5, 0.4, 0.1];

        Self {
            transitions: Arc::new(transitions),
            emissions: Arc::new(emissions),
            current_state_probs: Arc::new(Mutex::new(initial_probs)),
            is_paused: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Ok(mut p) = self.is_paused.lock() {
            *p = paused;
        }
    }

    pub fn discretize_flight_time(&self, ft: f64) -> usize {
        match ft {
            t if t < 80.0 => 0,
            t if t < 120.0 => 1,
            t if t < 160.0 => 2,
            t if t < 200.0 => 3,
            t if t < 300.0 => 4,
            t if t < 400.0 => 5,
            t if t < 500.0 => 6,
            t if t < 700.0 => 7,
            t if t < 1000.0 => 8,
            t if t < 1500.0 => 9,
            _ => 10,
        }
    }

    pub fn update(&self, ft: f64) {
        if let Ok(p) = self.is_paused.lock() {
            if *p { return; }
        }

        let obs = self.discretize_flight_time(ft);
        if obs >= 11 { return; } // Should not happen with logic above

        let mut current = self.current_state_probs.lock().unwrap();
        let old_probs = *current;
        let mut new_probs = [0.0; 3];

        let n_states = 3;

        // Forward Algorithm Step
        // new_prob[j] = (sum_{i} old_prob[i] * trans[i->j]) * emission[j][obs]
        
        let mut sum_prob = 0.0;

        for j in 0..n_states {
            let mut trans_sum = 0.0;
            for i in 0..n_states {
                // trans[i * 3 + j] is i -> j
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_probs[i] * t_prob;
            }

            // emission[j * 11 + obs]
            let e_prob = self.emissions[j * 11 + obs];
            
            new_probs[j] = trans_sum * e_prob;
            sum_prob += new_probs[j];
        }

        // Normalize
        if sum_prob > 0.0 {
            for j in 0..n_states {
                new_probs[j] /= sum_prob;
            }
        } else {
            // Fallback if probability vanishes (unlikely with these matrices)
            // Keep old probs or reset
        }

        *current = new_probs;
    }

    pub fn get_current_state(&self) -> HashMap<CognitiveState, f64> {
        let probs = self.current_state_probs.lock().unwrap();
        let mut map = HashMap::new();
        map.insert(CognitiveState::Flow, probs[0]);
        map.insert(CognitiveState::Incubation, probs[1]);
        map.insert(CognitiveState::Stuck, probs[2]);
        map
    }
}
