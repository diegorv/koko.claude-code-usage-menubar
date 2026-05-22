use std::sync::Mutex;
use std::time::Instant;

use crate::commands::UsagePayload;

#[derive(Default)]
pub struct PayloadCache {
    last_fetch: Mutex<Option<Instant>>,
    last_payload: Mutex<Option<UsagePayload>>,
}

impl PayloadCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the start of a fetch attempt so subsequent calls within the
    /// throttle window can return the cached payload.
    pub fn mark_fetch_start(&self) {
        if let Ok(mut last) = self.last_fetch.lock() {
            *last = Some(Instant::now());
        }
    }

    pub fn store(&self, payload: UsagePayload) {
        if let Ok(mut cached) = self.last_payload.lock() {
            *cached = Some(payload);
        }
    }

    pub fn get(&self) -> Option<UsagePayload> {
        self.last_payload.lock().ok()?.clone()
    }

    /// Returns the cached payload only if the last fetch started within
    /// `ttl_secs`. Used by `trigger_refresh` to honor the throttle.
    pub fn cached_if_fresh(&self, ttl_secs: u64) -> Option<UsagePayload> {
        let last = self.last_fetch.lock().ok()?;
        let started_at = (*last)?;
        if started_at.elapsed().as_secs() >= ttl_secs {
            return None;
        }
        drop(last);
        self.last_payload.lock().ok()?.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(status: &str) -> UsagePayload {
        UsagePayload {
            status: status.to_string(),
            session_percent: 10,
            session_resets_at: None,
            weekly_percent: 20,
            weekly_resets_at: None,
            models: vec![],
            last_updated_at: 0,
            error_message: None,
        }
    }

    #[test]
    fn cached_if_fresh_returns_none_when_empty() {
        let cache = PayloadCache::new();
        assert!(cache.cached_if_fresh(30).is_none());
    }

    #[test]
    fn cached_if_fresh_returns_payload_within_ttl() {
        let cache = PayloadCache::new();
        cache.mark_fetch_start();
        cache.store(payload("ok"));
        let got = cache.cached_if_fresh(30).expect("expected cached payload");
        assert_eq!(got.status, "ok");
    }

    #[test]
    fn cached_if_fresh_returns_none_after_ttl_expires() {
        let cache = PayloadCache::new();
        cache.mark_fetch_start();
        cache.store(payload("ok"));
        // ttl_secs = 0 → any positive elapsed time counts as expired.
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(cache.cached_if_fresh(0).is_none());
    }

    #[test]
    fn store_overwrites_previous() {
        let cache = PayloadCache::new();
        cache.store(payload("first"));
        cache.store(payload("second"));
        assert_eq!(cache.get().unwrap().status, "second");
    }

    #[test]
    fn get_returns_none_when_never_stored() {
        let cache = PayloadCache::new();
        assert!(cache.get().is_none());
    }
}
