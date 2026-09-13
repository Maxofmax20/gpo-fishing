use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::events::{Lifetime, Stats};

#[derive(Debug)]
pub struct Session {
    pub fish: u32,
    pub failed: u32,
    pub fruits: u32,
    pub bait_purchased: u32,
    pub restarts: u32,
    pub last_fruit: Option<String>,
    pub last_spawn: Option<String>,
    pub last_fish: Option<String>,
    pub since_progress_webhook: u32,
    pub since_purchase: u32,
    pub last_spawn_alert: Option<Instant>,
    pub pity_fruit: u32,
    pub pity_legendary: u32,
    outcomes: VecDeque<bool>,
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    base: Lifetime,
}

impl Session {
    pub fn new() -> Self {
        Self::with_base(Lifetime::default())
    }

    pub fn with_base(base: Lifetime) -> Self {
        Self {
            fish: 0,
            failed: 0,
            fruits: 0,
            bait_purchased: 0,
            restarts: 0,
            last_fruit: None,
            last_spawn: None,
            last_fish: None,
            since_progress_webhook: 0,
            since_purchase: 0,
            last_spawn_alert: None,
            pity_fruit: base.pity_fruit,
            pity_legendary: base.pity_legendary,
            outcomes: VecDeque::with_capacity(10),
            started: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
            base,
        }
    }

    pub fn lifetime(&self) -> Lifetime {
        let total_fish = self.base.fish + self.fish;
        Lifetime {
            fish: total_fish,
            failed: self.base.failed + self.failed,
            fruits: self.base.fruits + self.fruits,
            bait_purchased: self.base.bait_purchased + self.bait_purchased,
            runtime_s: self.base.runtime_s + self.runtime().as_secs(),
            sessions: self.base.sessions,
            last_fruit: self.last_fruit.clone().or_else(|| self.base.last_fruit.clone()),
            last_spawn: self.last_spawn.clone().or_else(|| self.base.last_spawn.clone()),
            last_fish: self.last_fish.clone().or_else(|| self.base.last_fish.clone()),
            pity_fruit: self.pity_fruit,
            pity_legendary: self.pity_legendary,
            estimated_peli: (total_fish as u64) * 95,
        }
    }

    pub fn next_session(&self) -> Session {
        let mut base = self.lifetime();
        base.sessions += 1;
        Session::with_base(base)
    }

    pub fn record(&mut self, caught: bool) {
        if self.outcomes.len() == 10 {
            self.outcomes.pop_front();
        }
        self.outcomes.push_back(caught);
        if caught {
            self.fish += 1;
            self.since_progress_webhook += 1;
            self.since_purchase += 1;
            self.pity_fruit += 1;
            self.pity_legendary += 1;
        }
    }

    pub fn record_failed(&mut self) {
        self.record(false);
        self.failed += 1;
    }

    pub fn success_rate(&self) -> f32 {
        if self.outcomes.is_empty() {
            return 0.8;
        }
        self.outcomes.iter().filter(|&&c| c).count() as f32 / self.outcomes.len() as f32
    }

    pub fn adaptive_timeout(&self, base: f32) -> f32 {
        let r = self.success_rate();
        if r > 0.7 {
            base * 1.3
        } else if r < 0.4 {
            base * 0.7
        } else {
            base
        }
    }

    pub fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    pub fn resume(&mut self) {
        if let Some(p) = self.paused_at.take() {
            self.paused_total += p.elapsed();
        }
    }

    pub fn runtime(&self) -> Duration {
        let end = self.paused_at.unwrap_or_else(Instant::now);
        end.duration_since(self.started).saturating_sub(self.paused_total)
    }

    pub fn stats(&self) -> Stats {
        let runtime_s = self.runtime().as_secs();
        let fish_per_hour = if runtime_s >= 30 {
            (self.fish as f32 / runtime_s as f32) * 3600.0
        } else {
            0.0
        };
        let estimated_peli = (self.fish as u64) * 95;

        Stats {
            fish: self.fish,
            failed: self.failed,
            fruits: self.fruits,
            bait_purchased: self.bait_purchased,
            success_rate: self.success_rate(),
            runtime_s,
            restarts: self.restarts,
            last_fruit: self.last_fruit.clone(),
            last_spawn: self.last_spawn.clone(),
            last_fish: self.last_fish.clone(),
            pity_fruit: self.pity_fruit,
            pity_legendary: self.pity_legendary,
            fish_per_hour,
            estimated_peli,
            since_purchase: self.since_purchase,
            since_progress: self.since_progress_webhook,
            total: self.lifetime(),
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_timeout_scales() {
        let mut s = Session::new();
        for _ in 0..10 {
            s.record(true);
        }
        assert!((s.adaptive_timeout(10.0) - 13.0).abs() < 0.01);
        for _ in 0..10 {
            s.record(false);
        }
        assert!((s.adaptive_timeout(10.0) - 7.0).abs() < 0.01);
    }
}
