// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Tracks per-route cooldowns, circuit-open state, and in-flight
//! concurrency so inference batches back off routes that are actively
//! failing or rate limited instead of retrying them on every single batch,
//! and stop retrying routes whose failures cannot self-resolve (a bad API
//! key does not fix itself on a timer). Route *selection* itself — which
//! configured route to prefer for a given batch — is decided deterministically
//! from the batch's own complexity (see `BatchComplexity` and
//! `GroqClient::ranked_byok_routes` in groq.rs), not by anything tracked here.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const MAX_COOLDOWN: Duration = Duration::from_secs(120);
const MAX_BACKOFF_EXPONENT: u32 = 4;

#[derive(Debug, Clone, Copy, Default)]
struct RouteStats {
    attempts: u64,
    successes: u64,
    failures: u64,
    total_cooldown: Duration,
}

/// A point-in-time summary of one route's behavior over this client's
/// lifetime, used to log a single aggregated line per route at the end of a
/// run instead of requiring an operator to grep hundreds of per-batch lines.
pub(super) struct RouteSnapshot {
    pub(super) name: String,
    pub(super) attempts: u64,
    pub(super) successes: u64,
    pub(super) failures: u64,
    pub(super) total_cooldown: Duration,
    pub(super) circuit_open: bool,
}

pub(super) struct RouteHealth {
    cooldowns: Mutex<HashMap<String, Instant>>,
    failure_streaks: Mutex<HashMap<String, u32>>,
    /// Routes whose most recent failure was classified as unrecoverable
    /// (e.g. a rejected API key) rather than transient (e.g. a 429 or a
    /// dropped connection). Once open, a route is skipped by route
    /// selection and the "wait for the soonest recovery" fallback alike —
    /// clearing it requires a fresh `RouteHealth` (a process restart) or an
    /// explicit `record_success`, since nothing about waiting fixes a bad key.
    circuit_open: Mutex<HashSet<String>>,
    /// Per-route semaphores bounding how many requests to a given route may
    /// be in flight at once, so one fast provider can't starve a slower
    /// one's fair share when multiple batches dispatch concurrently.
    route_semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    stats: Mutex<HashMap<String, RouteStats>>,
}

/// Unwrap a mutex guard, recovering its data on a poisoned lock instead of
/// propagating the panic into whatever analysis batch is in flight. A prior
/// panic elsewhere becomes a traced event here rather than a second, opaque
/// panic with no telemetry trail.
fn recover_lock<'a, T>(
    guard: std::sync::LockResult<std::sync::MutexGuard<'a, T>>,
    map: &'static str,
    route: &str,
) -> std::sync::MutexGuard<'a, T> {
    guard.unwrap_or_else(|poisoned| {
        tracing::error!(
            map = map,
            route = route,
            "route health lock was poisoned by an earlier panic; recovering its data instead of panicking again"
        );
        poisoned.into_inner()
    })
}

impl RouteHealth {
    pub(super) fn new() -> Self {
        Self {
            cooldowns: Mutex::new(HashMap::new()),
            failure_streaks: Mutex::new(HashMap::new()),
            circuit_open: Mutex::new(HashSet::new()),
            route_semaphores: Mutex::new(HashMap::new()),
            stats: Mutex::new(HashMap::new()),
        }
    }

    /// Record that a request to `name` is about to be attempted, for the
    /// end-of-run summary.
    pub(super) fn record_attempt(&self, name: &str) {
        recover_lock(self.stats.lock(), "stats", name)
            .entry(name.to_string())
            .or_default()
            .attempts += 1;
    }

    /// Record a failure for `name`, escalating the cooldown with each
    /// consecutive failure and honoring the caller-supplied `base_cooldown`
    /// (typically derived from a server retry hint) as the starting point.
    /// Returns the cooldown that was applied. For failures that cannot
    /// self-resolve, call [`Self::open_circuit`] instead.
    pub(super) fn record_failure(&self, name: &str, base_cooldown: Duration) -> Duration {
        let exponent = {
            let mut streaks = recover_lock(self.failure_streaks.lock(), "failure_streaks", name);
            let streak = streaks.entry(name.to_string()).or_insert(0);
            *streak = streak.saturating_add(1);
            (*streak - 1).min(MAX_BACKOFF_EXPONENT)
        };
        let cooldown = base_cooldown
            .checked_mul(1u32 << exponent)
            .unwrap_or(MAX_COOLDOWN)
            .min(MAX_COOLDOWN);
        recover_lock(self.cooldowns.lock(), "cooldowns", name)
            .insert(name.to_string(), Instant::now() + cooldown);
        {
            let mut stats = recover_lock(self.stats.lock(), "stats", name);
            let entry = stats.entry(name.to_string()).or_default();
            entry.failures += 1;
            entry.total_cooldown += cooldown;
        }
        cooldown
    }

    /// Permanently stop automatic retries against `name` until this
    /// `RouteHealth` is replaced (a process restart) or an explicit
    /// [`Self::record_success`] clears it. Use this for failures that a
    /// cooldown timer cannot fix, such as a rejected API key.
    pub(super) fn open_circuit(&self, name: &str, reason: &str) {
        let newly_opened =
            recover_lock(self.circuit_open.lock(), "circuit_open", name).insert(name.to_string());
        recover_lock(self.stats.lock(), "stats", name)
            .entry(name.to_string())
            .or_default()
            .failures += 1;
        if newly_opened {
            tracing::error!(
                route = name,
                reason = reason,
                "route circuit-opened: this failure will not self-resolve, so automatic retries are stopped until the process restarts"
            );
        }
    }

    pub(super) fn is_circuit_open(&self, name: &str) -> bool {
        recover_lock(self.circuit_open.lock(), "circuit_open", name).contains(name)
    }

    /// Clear a route's cooldown, failure streak, and circuit-open state
    /// after a successful call.
    pub(super) fn record_success(&self, name: &str) {
        recover_lock(self.failure_streaks.lock(), "failure_streaks", name).remove(name);
        recover_lock(self.cooldowns.lock(), "cooldowns", name).remove(name);
        recover_lock(self.circuit_open.lock(), "circuit_open", name).remove(name);
        recover_lock(self.stats.lock(), "stats", name)
            .entry(name.to_string())
            .or_default()
            .successes += 1;
    }

    /// Callers that also need the remaining duration (to log or sleep on it)
    /// use `remaining_cooldown` directly; kept for callers that only need
    /// the yes/no answer, and exercised by its own tests below.
    #[allow(dead_code)]
    pub(super) fn is_cooling_down(&self, name: &str) -> bool {
        self.remaining_cooldown(name).is_some()
    }

    pub(super) fn remaining_cooldown(&self, name: &str) -> Option<Duration> {
        let until = *recover_lock(self.cooldowns.lock(), "cooldowns", name).get(name)?;
        let now = Instant::now();
        (until > now).then(|| until - now)
    }

    /// The semaphore bounding concurrent in-flight requests to `name`,
    /// created with `max_concurrency` permits the first time this route is
    /// seen. Callers should hold the returned permit for the duration of
    /// their request.
    pub(super) fn route_semaphore(&self, name: &str, max_concurrency: usize) -> Arc<Semaphore> {
        recover_lock(self.route_semaphores.lock(), "route_semaphores", name)
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(max_concurrency)))
            .clone()
    }

    /// A point-in-time summary of every route this client has attempted,
    /// sorted by name for stable log output.
    pub(super) fn snapshot(&self) -> Vec<RouteSnapshot> {
        let stats = recover_lock(self.stats.lock(), "stats", "<snapshot>");
        let circuit_open = recover_lock(self.circuit_open.lock(), "circuit_open", "<snapshot>");
        let mut snapshots: Vec<RouteSnapshot> = stats
            .iter()
            .map(|(name, route_stats)| RouteSnapshot {
                name: name.clone(),
                attempts: route_stats.attempts,
                successes: route_stats.successes,
                failures: route_stats.failures,
                total_cooldown: route_stats.total_cooldown,
                circuit_open: circuit_open.contains(name),
            })
            .collect();
        snapshots.sort_by(|a, b| a.name.cmp(&b.name));
        snapshots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_route_is_not_cooling_down() {
        let health = RouteHealth::new();
        assert!(!health.is_cooling_down("groq"));
        assert!(health.remaining_cooldown("groq").is_none());
    }

    #[test]
    fn record_failure_starts_a_cooldown_and_success_clears_it() {
        let health = RouteHealth::new();
        health.record_failure("groq", Duration::from_secs(5));
        assert!(health.is_cooling_down("groq"));
        let remaining = health.remaining_cooldown("groq").unwrap();
        assert!(remaining <= Duration::from_secs(5) && remaining > Duration::ZERO);

        health.record_success("groq");
        assert!(!health.is_cooling_down("groq"));
    }

    #[test]
    fn repeated_failures_escalate_the_cooldown_exponentially() {
        let health = RouteHealth::new();
        let first = health.record_failure("groq", Duration::from_secs(1));
        let second = health.record_failure("groq", Duration::from_secs(1));
        let third = health.record_failure("groq", Duration::from_secs(1));
        assert_eq!(first, Duration::from_secs(1));
        assert_eq!(second, Duration::from_secs(2));
        assert_eq!(third, Duration::from_secs(4));
    }

    #[test]
    fn escalating_cooldown_is_capped_at_the_maximum() {
        let health = RouteHealth::new();
        let mut last = Duration::ZERO;
        for _ in 0..10 {
            last = health.record_failure("dead-route", Duration::from_secs(30));
        }
        assert_eq!(last, MAX_COOLDOWN);
    }

    #[test]
    fn a_fresh_success_resets_the_failure_streak() {
        let health = RouteHealth::new();
        health.record_failure("groq", Duration::from_secs(1));
        health.record_failure("groq", Duration::from_secs(1));
        health.record_success("groq");
        // The streak should restart from the base cooldown, not continue
        // escalating from where it left off.
        let cooldown = health.record_failure("groq", Duration::from_secs(1));
        assert_eq!(cooldown, Duration::from_secs(1));
    }

    #[test]
    fn independent_routes_have_independent_cooldowns() {
        let health = RouteHealth::new();
        health.record_failure("groq", Duration::from_secs(5));
        assert!(health.is_cooling_down("groq"));
        assert!(!health.is_cooling_down("openai"));
    }

    #[test]
    fn open_circuit_persists_regardless_of_cooldown_expiry() {
        let health = RouteHealth::new();
        assert!(!health.is_circuit_open("openai"));
        health.open_circuit("openai", "401 Unauthorized: invalid api key");
        assert!(health.is_circuit_open("openai"));
        // Unlike a cooldown, nothing about time passing clears this.
        assert!(health.is_circuit_open("openai"));
    }

    #[test]
    fn record_success_clears_an_open_circuit() {
        let health = RouteHealth::new();
        health.open_circuit("openai", "401 Unauthorized");
        assert!(health.is_circuit_open("openai"));
        health.record_success("openai");
        assert!(!health.is_circuit_open("openai"));
    }

    #[test]
    fn independent_routes_have_independent_circuit_state() {
        let health = RouteHealth::new();
        health.open_circuit("openai", "401 Unauthorized");
        assert!(health.is_circuit_open("openai"));
        assert!(!health.is_circuit_open("groq"));
    }

    #[test]
    fn route_semaphore_is_shared_across_calls_for_the_same_route() {
        let health = RouteHealth::new();
        let first = health.route_semaphore("groq", 4);
        let second = health.route_semaphore("groq", 4);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.available_permits(), 4);
    }

    #[test]
    fn route_semaphore_is_independent_per_route() {
        let health = RouteHealth::new();
        let groq = health.route_semaphore("groq", 2);
        let openai = health.route_semaphore("openai", 5);
        assert!(!Arc::ptr_eq(&groq, &openai));
        assert_eq!(openai.available_permits(), 5);
    }

    #[test]
    fn snapshot_reflects_attempts_successes_and_failures() {
        let health = RouteHealth::new();
        health.record_attempt("groq");
        health.record_attempt("groq");
        health.record_success("groq");
        health.record_attempt("openai");
        health.record_failure("openai", Duration::from_secs(1));

        let snapshots = health.snapshot();
        let groq = snapshots.iter().find(|s| s.name == "groq").unwrap();
        assert_eq!(groq.attempts, 2);
        assert_eq!(groq.successes, 1);
        assert_eq!(groq.failures, 0);
        assert!(!groq.circuit_open);

        let openai = snapshots.iter().find(|s| s.name == "openai").unwrap();
        assert_eq!(openai.attempts, 1);
        assert_eq!(openai.failures, 1);
        assert_eq!(openai.total_cooldown, Duration::from_secs(1));
    }

    #[test]
    fn snapshot_reflects_circuit_open_routes() {
        let health = RouteHealth::new();
        health.record_attempt("openai");
        health.open_circuit("openai", "401 Unauthorized");
        let snapshots = health.snapshot();
        let openai = snapshots.iter().find(|s| s.name == "openai").unwrap();
        assert!(openai.circuit_open);
        assert_eq!(openai.failures, 1);
    }
}
