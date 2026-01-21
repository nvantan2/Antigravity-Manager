use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

static WARMUP_HISTORY: Lazy<Mutex<HashMap<String, i64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn check_cooldown(key: &str, cooldown_secs: i64) -> bool {
    let now_ts = chrono::Utc::now().timestamp();
    let history = WARMUP_HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    match history.get(key) {
        Some(last_ts) => now_ts.saturating_sub(*last_ts) < cooldown_secs,
        None => false,
    }
}

pub fn record_warmup_history(key: &str, timestamp: i64) {
    let mut history = WARMUP_HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    history.insert(key.to_string(), timestamp);
}
