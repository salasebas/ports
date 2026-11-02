use std::collections::VecDeque;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

const BUCKET_MS: u64 = 10_000;
const BUCKETS: u64 = 60;
const MB: u64 = 1024 * 1024;
const GROWTH_WINDOW_MINUTES: [u64; 2] = [5, 10];
const GROWTH_MIN_BYTES: RangeInclusive<u64> = 10 * MB..=4096 * MB;
const LEAK_MIN_GROWTH_RATIO: f64 = 0.1;

pub const DEFAULT_IDLE_AFTER_MS: u64 = 5 * 60_000;
pub const ACTIVE_CPU_PERCENT: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Idle,
    Attention,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthRule {
    pub window_minutes: u64,
    pub min_growth_bytes: u64,
}

impl Default for GrowthRule {
    fn default() -> Self {
        Self {
            window_minutes: 10,
            min_growth_bytes: 50 * MB,
        }
    }
}

impl GrowthRule {
    pub fn is_valid(&self) -> bool {
        GROWTH_WINDOW_MINUTES.contains(&self.window_minutes)
            && GROWTH_MIN_BYTES.contains(&self.min_growth_bytes)
    }

    fn buckets(&self) -> u64 {
        self.window_minutes * 60_000 / BUCKET_MS
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Growth {
    pub bytes: u64,
    pub minutes: u64,
}

#[derive(Debug)]
struct Bucket {
    slot: u64,
    memory: u64,
    cpu_total: f64,
    samples: u32,
}

#[derive(Debug, Default)]
pub struct History {
    buckets: VecDeque<Bucket>,
}

impl History {
    pub fn record(&mut self, now_ms: u64, memory: u64, cpu: f64) {
        let slot = now_ms / BUCKET_MS;
        if self.buckets.back().is_some_and(|bucket| slot < bucket.slot) {
            self.buckets.clear();
        }
        match self.buckets.back_mut() {
            Some(bucket) if bucket.slot == slot => {
                bucket.memory = memory;
                bucket.cpu_total += cpu;
                bucket.samples += 1;
            }
            _ => self.buckets.push_back(Bucket {
                slot,
                memory,
                cpu_total: cpu,
                samples: 1,
            }),
        }
        while self
            .buckets
            .front()
            .is_some_and(|b| b.slot + BUCKETS <= slot)
        {
            self.buckets.pop_front();
        }
    }

    pub fn memory(&self) -> Vec<u64> {
        self.buckets.iter().map(|b| b.memory).collect()
    }

    pub fn growth(&self, rule: GrowthRule) -> Option<Growth> {
        let last = self.buckets.back()?.slot;
        let window: Vec<_> = self
            .buckets
            .iter()
            .filter(|bucket| bucket.slot + rule.buckets() > last)
            .collect();
        let span = (last - window.first()?.slot + 1) as usize;
        let memory: Vec<u64> = window.iter().map(|bucket| bucket.memory).collect();
        is_leaking(&memory, span, rule).then(|| Growth {
            bytes: memory[memory.len() - 1].saturating_sub(memory[0]),
            minutes: rule.window_minutes,
        })
    }

    pub fn cpu(&self) -> Vec<f64> {
        self.buckets
            .iter()
            .map(|b| b.cpu_total / f64::from(b.samples))
            .collect()
    }
}

fn is_leaking(memory: &[u64], span: usize, rule: GrowthRule) -> bool {
    const SLICES: usize = 6;
    if span < rule.buckets() as usize * 9 / 10 || memory.len() < SLICES {
        return false;
    }
    let means = slice_means(memory, SLICES);
    let growth = means[SLICES - 1] - means[0];
    let grew_enough =
        growth >= (rule.min_growth_bytes as f64).max(means[0] * LEAK_MIN_GROWTH_RATIO);
    let rising_slices = means.windows(2).filter(|pair| pair[1] > pair[0]).count();
    let rose_through_garbage_collections = rising_slices >= SLICES - 2;
    grew_enough && rose_through_garbage_collections
}

fn slice_means(memory: &[u64], slices: usize) -> Vec<f64> {
    (0..slices)
        .map(|i| &memory[i * memory.len() / slices..(i + 1) * memory.len() / slices])
        .map(|slice| slice.iter().map(|&m| m as f64).sum::<f64>() / slice.len() as f64)
        .collect()
}

pub fn status(leaking: bool, now_ms: u64, last_active_at: u64, idle_after_ms: u64) -> Status {
    if leaking {
        Status::Attention
    } else if now_ms.saturating_sub(last_active_at) >= idle_after_ms {
        Status::Idle
    } else {
        Status::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_flagged(memory: &[u64], span: usize) -> bool {
        is_leaking(memory, span, GrowthRule::default())
    }

    #[test]
    fn folds_samples_into_ten_second_buckets() {
        let mut history = History::default();
        history.record(0, 100, 2.0);
        history.record(5_000, 120, 4.0);
        history.record(10_000, 130, 1.0);
        assert_eq!(history.memory(), vec![120, 130]);
        assert_eq!(history.cpu(), vec![3.0, 1.0]);
    }

    #[test]
    fn keeps_only_the_last_ten_minutes() {
        let mut history = History::default();
        for second in (0..=900).step_by(10) {
            history.record(second * 1000, second, 0.0);
        }
        let memory = history.memory();
        assert_eq!(memory.len(), 60);
        assert_eq!(memory.first(), Some(&310));
    }

    fn window(f: impl Fn(u64) -> u64) -> Vec<u64> {
        (0..60).map(f).collect()
    }

    #[test]
    fn flags_memory_that_grows_steadily() {
        let growing = window(|i| 400 * MB + i * 5 * MB + (i % 4) * 8 * MB);
        assert!(is_flagged(&growing, 60));
        let sparse: Vec<u64> = growing.iter().step_by(3).copied().collect();
        assert!(is_flagged(&sparse, 60));
    }

    #[test]
    fn does_not_flag_flat_noisy_or_one_off_growth() {
        let flat = window(|i| 400 * MB + (i % 5) * 20 * MB);
        let one_jump = window(|i| if i < 30 { 200 * MB } else { 900 * MB });
        let small = window(|i| 400 * MB + i * MB / 4);
        assert!(!is_flagged(&flat, 60));
        assert!(!is_flagged(&one_jump, 60));
        assert!(!is_flagged(&small, 60));
    }

    #[test]
    fn waits_for_a_full_window_before_flagging() {
        let growing: Vec<u64> = (0..20).map(|i| 400 * MB + i * 50 * MB).collect();
        assert!(!is_flagged(&growing, 20));
    }

    #[test]
    fn idles_after_five_quiet_minutes() {
        let now = 60 * 60_000;
        let idle_after = DEFAULT_IDLE_AFTER_MS;
        assert_eq!(
            status(false, now, now - 30 * 60_000, idle_after),
            Status::Idle
        );
        assert_eq!(
            status(false, now, now - 60_000, idle_after),
            Status::Running
        );
        assert_eq!(
            status(true, now, now - 30 * 60_000, idle_after),
            Status::Attention
        );
        assert_eq!(
            status(false, now, now - 30 * 60_000, 60 * 60_000),
            Status::Running
        );
    }

    #[test]
    fn includes_recovery_at_the_end_of_a_sparse_window() {
        let mut memory: Vec<u64> = (0..6).map(|i| (100 + i * 25) * MB).collect();
        memory.extend([100 * MB; 5]);
        assert!(!is_flagged(&memory, 60));
    }

    fn recorded(memory_at: impl Fn(u64) -> u64) -> History {
        let mut history = History::default();
        for bucket in 0..60 {
            history.record(bucket * BUCKET_MS, memory_at(bucket), 0.0);
        }
        history
    }

    #[test]
    fn a_higher_minimum_growth_ignores_smaller_growth() {
        let history = recorded(|bucket| 400 * MB + bucket * 2 * MB);
        let growth = history.growth(GrowthRule::default()).unwrap();
        assert_eq!(growth.bytes, 118 * MB);
        assert_eq!(growth.minutes, 10);
        let stricter = GrowthRule {
            min_growth_bytes: 200 * MB,
            ..GrowthRule::default()
        };
        assert_eq!(history.growth(stricter), None);
    }

    #[test]
    fn a_shorter_window_judges_and_reports_only_the_recent_growth() {
        let history = recorded(|bucket| 400 * MB + bucket.saturating_sub(30) * 3 * MB);
        let recent = GrowthRule {
            window_minutes: 5,
            ..GrowthRule::default()
        };
        assert_eq!(
            history.growth(recent),
            Some(Growth {
                bytes: 87 * MB,
                minutes: 5,
            })
        );
    }

    #[test]
    fn only_the_offered_growth_rules_are_valid() {
        assert!(GrowthRule::default().is_valid());
        for invalid in [
            GrowthRule {
                window_minutes: 20,
                ..GrowthRule::default()
            },
            GrowthRule {
                min_growth_bytes: MB,
                ..GrowthRule::default()
            },
        ] {
            assert!(!invalid.is_valid());
        }
    }

    #[test]
    fn starts_a_new_window_after_the_clock_moves_backwards() {
        let mut history = History::default();
        history.record(1_000_000, 200, 1.0);
        history.record(900_000, 100, 0.0);
        assert_eq!(history.memory(), vec![100]);
    }
}
