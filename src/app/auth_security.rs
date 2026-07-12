use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const BLOCK_DURATION: Duration = Duration::from_secs(15 * 60);
const MAX_FAILURES_PER_WINDOW: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginRateLimitDecision {
    Allowed,
    RetryAfter(u64),
}

#[derive(Debug, Clone)]
pub(crate) struct LoginRateLimiter {
    attempts: Arc<Mutex<HashMap<IpAddr, LoginAttemptState>>>,
}

#[derive(Debug, Clone, Copy)]
struct LoginAttemptState {
    window_started: Instant,
    failures: u32,
    blocked_until: Option<Instant>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl LoginRateLimiter {
    pub(crate) fn check(&self, peer: IpAddr) -> LoginRateLimitDecision {
        self.check_at(peer, Instant::now())
    }

    pub(crate) fn record_failure(&self, peer: IpAddr) {
        self.record_failure_at(peer, Instant::now());
    }

    pub(crate) fn record_success(&self, peer: IpAddr) {
        self.attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&peer);
    }

    fn check_at(&self, peer: IpAddr, now: Instant) -> LoginRateLimitDecision {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        attempts.retain(|_, state| {
            state
                .blocked_until
                .is_some_and(|blocked_until| blocked_until > now)
                || now.saturating_duration_since(state.window_started)
                    <= FAILURE_WINDOW + BLOCK_DURATION
        });
        let Some(state) = attempts.get_mut(&peer) else {
            return LoginRateLimitDecision::Allowed;
        };
        if let Some(blocked_until) = state.blocked_until {
            if blocked_until > now {
                return LoginRateLimitDecision::RetryAfter(
                    blocked_until
                        .saturating_duration_since(now)
                        .as_secs()
                        .max(1),
                );
            }
            state.blocked_until = None;
            state.failures = 0;
            state.window_started = now;
        } else if now.saturating_duration_since(state.window_started) > FAILURE_WINDOW {
            state.failures = 0;
            state.window_started = now;
        }
        LoginRateLimitDecision::Allowed
    }

    fn record_failure_at(&self, peer: IpAddr, now: Instant) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = attempts.entry(peer).or_insert(LoginAttemptState {
            window_started: now,
            failures: 0,
            blocked_until: None,
        });
        if now.saturating_duration_since(state.window_started) > FAILURE_WINDOW {
            state.window_started = now;
            state.failures = 0;
            state.blocked_until = None;
        }
        state.failures = state.failures.saturating_add(1);
        if state.failures >= MAX_FAILURES_PER_WINDOW {
            state.blocked_until = Some(now + BLOCK_DURATION);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn login_rate_limiter_blocks_repeated_failures_and_resets_after_success() {
        let limiter = LoginRateLimiter::default();
        let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let now = Instant::now();

        for _ in 0..MAX_FAILURES_PER_WINDOW {
            assert_eq!(limiter.check_at(peer, now), LoginRateLimitDecision::Allowed);
            limiter.record_failure_at(peer, now);
        }
        assert_eq!(
            limiter.check_at(peer, now),
            LoginRateLimitDecision::RetryAfter(BLOCK_DURATION.as_secs())
        );

        limiter.record_success(peer);
        assert_eq!(limiter.check_at(peer, now), LoginRateLimitDecision::Allowed);
    }

    #[test]
    fn login_rate_limiter_isolated_peers_and_expired_windows_do_not_share_failures() {
        let limiter = LoginRateLimiter::default();
        let first = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let second = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
        let now = Instant::now();

        for _ in 0..MAX_FAILURES_PER_WINDOW {
            limiter.record_failure_at(first, now);
        }
        assert_eq!(
            limiter.check_at(second, now),
            LoginRateLimitDecision::Allowed
        );
        assert_eq!(
            limiter.check_at(first, now + BLOCK_DURATION + Duration::from_secs(1)),
            LoginRateLimitDecision::Allowed
        );
    }
}
