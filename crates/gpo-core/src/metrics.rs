//! Fixed-size, allocation-free controller measurements; snapshots allocate off-loop.
use std::sync::atomic::{AtomicU64,Ordering};
use std::time::Duration;
#[derive(Default)]
pub struct ControlMetrics {
    samples:[AtomicU64;32], frames:AtomicU64, missed:AtomicU64,
}
#[derive(Debug,Clone,serde::Serialize,serde::Deserialize)]
pub struct MetricsSnapshot {
    pub frames:u64, pub missed_deadlines:u64, pub window_samples:usize,
    pub processing_p50_us:Option<u64>, pub processing_p95_us:Option<u64>, pub processing_max_us:Option<u64>,
}
impl ControlMetrics {
    pub fn record(&self,processing:Duration,period:Duration) {
        let index=self.frames.fetch_add(1,Ordering::Relaxed);
        let us=(processing.as_micros().min(u64::MAX as u128) as u64).max(1);
        self.samples[index as usize % self.samples.len()].store(us,Ordering::Relaxed);
        if processing>period {self.missed.fetch_add(1,Ordering::Relaxed);}
    }
    pub fn snapshot(&self)->MetricsSnapshot {
        let mut values:Vec<_>=self.samples.iter().map(|s|s.load(Ordering::Relaxed)).filter(|v|*v>0).collect();
        values.sort_unstable();
        let percentile=|p:usize|values.get((values.len()*p).div_ceil(100).saturating_sub(1)).copied();
        MetricsSnapshot {frames:self.frames.load(Ordering::Relaxed),missed_deadlines:self.missed.load(Ordering::Relaxed),
            window_samples:values.len(),processing_p50_us:percentile(50),processing_p95_us:percentile(95),processing_max_us:values.last().copied()}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_snapshot_is_unknown_and_samples_are_bounded() {
        let metrics=ControlMetrics::default();
        assert!(metrics.snapshot().processing_p95_us.is_none());
        for n in 1..=100 {metrics.record(Duration::from_micros(n),Duration::from_micros(90));}
        let snapshot=metrics.snapshot();
        assert_eq!(snapshot.frames,100);assert_eq!(snapshot.window_samples,32);
        assert_eq!(snapshot.missed_deadlines,10);assert_eq!(snapshot.processing_max_us,Some(100));
        assert_eq!(snapshot.processing_p95_us,Some(99));
    }
}
