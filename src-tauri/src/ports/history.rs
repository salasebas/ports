//! Ten minutes of samples per server and the statuses derived from them.

use std::collections::VecDeque;

use serde::Serialize;

const BUCKET_MS: u64 = 10_000;
/// Ten minutes of ten-second buckets.
const BUCKETS: u64 = 60;
/// A leak is only called over (nearly) the whole window.
const LEAK_MIN_BUCKETS: usize = 54;
const LEAK_MIN_GROWTH_BYTES: f64 = 50.0 * 1024.0 * 1024.0;
const LEAK_MIN_GROWTH_RATIO: f64 = 0.1;

/// How long a server goes without connections or CPU before it counts as
/// idle, unless the user chose another threshold in the settings.
pub const DEFAULT_IDLE_AFTER_MS: u64 = 5 * 60_000;
/// Percent of one core; dev servers idle well below this, file watchers included.
pub const ACTIVE_CPU_PERCENT: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Idle,
    Attention,
}

#[derive(Debug)]
struct Bucket {
    slot: u64,
    memory: u64,
    cpu_total: f64,
    samples: u32,
}

/// Samples folded into ten-second buckets: the latest memory and the
/// average CPU of each bucket. Buckets older than ten minutes fall off, so
/// a gap while polling was paused shortens the history instead of stretching it.
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

    /// How many ten-second buckets the history covers, gaps included.
    pub fn span(&self) -> usize {
        match (self.buckets.front(), self.buckets.back()) {
            (Some(first), Some(last)) => (last.slot - first.slot + 1) as usize,
            _ => 0,
        }
    }

    pub fn cpu(&self) -> Vec<f64> {
        self.buckets
            .iter()
            .map(|b| b.cpu_total / f64::from(b.samples))
            .collect()
    }
}

/// Memory that grew by a meaningful amount and kept rising across a window
/// of `span` buckets. Comparing the means of six slices rides out garbage
/// collector sawtooths, and a one-off jump (a cache filling once) rises in a
/// single slice only.
pub fn is_leaking(memory: &[u64], span: usize) -> bool {
    const SLICES: usize = 6;
    if span < LEAK_MIN_BUCKETS || memory.len() < SLICES {
        return false;
    }
    let means: Vec<f64> = (0..SLICES)
        .map(|i| &memory[i * memory.len() / SLICES..(i + 1) * memory.len() / SLICES])
        .map(|slice| slice.iter().map(|&m| m as f64).sum::<f64>() / slice.len() as f64)
        .collect();
    let growth = means[SLICES - 1] - means[0];
    let rises = means.windows(2).filter(|pair| pair[1] > pair[0]).count();
    growth >= LEAK_MIN_GROWTH_BYTES.max(means[0] * LEAK_MIN_GROWTH_RATIO) && rises >= SLICES - 2
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

    const MB: u64 = 1024 * 1024;

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
        // +5 MB per bucket with a garbage-collector sawtooth on top.
        let growing = window(|i| 400 * MB + i * 5 * MB + (i % 4) * 8 * MB);
        assert!(is_leaking(&growing, 60));
        // Sampled every 30 s while Ports is hidden: fewer points, same ten minutes.
        let sparse: Vec<u64> = growing.iter().step_by(3).copied().collect();
        assert!(is_leaking(&sparse, 60));
    }

    #[test]
    fn does_not_flag_flat_noisy_or_one_off_growth() {
        let flat = window(|i| 400 * MB + (i % 5) * 20 * MB);
        let one_jump = window(|i| if i < 30 { 200 * MB } else { 900 * MB });
        let small = window(|i| 400 * MB + i * MB / 4);
        assert!(!is_leaking(&flat, 60));
        assert!(!is_leaking(&one_jump, 60));
        assert!(!is_leaking(&small, 60));
    }

    #[test]
    fn waits_for_a_full_window_before_flagging() {
        let growing: Vec<u64> = (0..20).map(|i| 400 * MB + i * 50 * MB).collect();
        assert!(!is_leaking(&growing, 20));
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
        // A longer threshold keeps the same server running.
        assert_eq!(
            status(false, now, now - 30 * 60_000, 60 * 60_000),
            Status::Running
        );
    }

    #[test]
    fn includes_recovery_at_the_end_of_a_sparse_window() {
        let mut memory: Vec<u64> = (0..6).map(|i| (100 + i * 25) * MB).collect();
        memory.extend([100 * MB; 5]);
        assert!(!is_leaking(&memory, 60));
    }

    #[test]
    fn starts_a_new_window_after_the_clock_moves_backwards() {
        let mut history = History::default();
        history.record(1_000_000, 200, 1.0);
        history.record(900_000, 100, 0.0);
        assert_eq!(history.span(), 1);
        assert_eq!(history.memory(), vec![100]);
    }
}
