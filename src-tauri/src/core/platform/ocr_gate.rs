//! One outstanding OCR operation, including requests whose cancellation is asynchronous.
//! A timed-out operation is retained until terminal; late text is never returned as a new frame.
use std::time::{Duration, Instant};

pub enum OcrPoll {
    Pending,
    Finished(Result<String, String>),
}

pub trait OcrRequest: Send {
    fn poll(&self) -> Result<OcrPoll, String>;
    fn cancel(&self);
}

#[derive(Default)]
pub struct OcrRequestGate {
    pending: Option<Box<dyn OcrRequest>>,
}

impl OcrRequestGate {
    pub fn run(
        &mut self,
        create: impl FnOnce() -> Result<Box<dyn OcrRequest>, String>,
        timeout: Duration,
    ) -> Result<String, String> {
        if let Some(previous) = &self.pending {
            match previous.poll()? {
                OcrPoll::Pending => {
                    return Err("Previous OCR request is still finishing cancellation".into())
                }
                OcrPoll::Finished(_) => {
                    self.pending.take();
                }
            }
        }
        self.pending = Some(create()?);
        let started = Instant::now();
        loop {
            let request = self.pending.as_ref().expect("active OCR request");
            match request.poll() {
                Ok(OcrPoll::Finished(result)) => {
                    self.pending.take();
                    return result;
                }
                Ok(OcrPoll::Pending) => {}
                Err(error) => {
                    // A status-query failure does not prove the operation finished.
                    // Cancel, but retain its handle so another call cannot multiply it.
                    request.cancel();
                    return Err(error);
                }
            }
            let Some(remaining) = timeout
                .checked_sub(started.elapsed())
                .filter(|left| !left.is_zero())
            else {
                request.cancel();
                return Err("OCR request timed out; cancellation requested".into());
            };
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct Fake {
        polls: PollSequence,
        cancelled: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
    }
    impl OcrRequest for Fake {
        fn poll(&self) -> Result<OcrPoll, String> {
            let mut polls = self.polls.lock().unwrap();
            let value = if polls.is_empty() {
                Ok(None)
            } else {
                polls.remove(0)
            };
            value.map(|text| match text {
                Some(text) => OcrPoll::Finished(Ok(text)),
                None => OcrPoll::Pending,
            })
        }
        fn cancel(&self) {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    type PollSequence = Arc<Mutex<Vec<Result<Option<String>, String>>>>;
    type FakeHandle = (
        Box<dyn OcrRequest>,
        PollSequence,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    );
    fn fake(values: Vec<Result<Option<String>, String>>) -> FakeHandle {
        let polls = Arc::new(Mutex::new(values));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        (
            Box::new(Fake {
                polls: polls.clone(),
                cancelled: cancelled.clone(),
                dropped: dropped.clone(),
            }),
            polls,
            cancelled,
            dropped,
        )
    }
    #[test]
    fn completed_request_returns_its_own_text_and_releases_handle() {
        let mut gate = OcrRequestGate::default();
        let (request, _, cancelled, dropped) = fake(vec![Ok(Some("current frame".into()))]);
        assert_eq!(
            gate.run(|| Ok(request), Duration::from_millis(50)).unwrap(),
            "current frame"
        );
        assert_eq!(cancelled.load(Ordering::SeqCst), 0);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn timeout_retains_request_until_asynchronous_cancellation_finishes() {
        let mut gate = OcrRequestGate::default();
        let (request, polls, cancelled, dropped) = fake(vec![]);
        assert!(gate
            .run(|| Ok(request), Duration::ZERO)
            .unwrap_err()
            .contains("timed out"));
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert!(gate
            .run(
                || panic!("must not create another OCR operation"),
                Duration::ZERO
            )
            .unwrap_err()
            .contains("still finishing"));
        polls
            .lock()
            .unwrap()
            .push(Ok(Some("late stale text".into())));
        let (next, _, _, _) = fake(vec![Ok(Some("new frame".into()))]);
        assert_eq!(
            gate.run(|| Ok(next), Duration::from_millis(50)).unwrap(),
            "new frame"
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn unknown_operation_status_cannot_start_unbounded_work() {
        let mut gate = OcrRequestGate::default();
        let (request, polls, cancelled, dropped) = fake(vec![Err("status unavailable".into())]);
        assert_eq!(
            gate.run(|| Ok(request), Duration::ZERO).unwrap_err(),
            "status unavailable"
        );
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        polls
            .lock()
            .unwrap()
            .push(Err("status still unavailable".into()));
        assert!(gate
            .run(|| panic!("must retain uncertain operation"), Duration::ZERO)
            .is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn creation_failure_does_not_poison_later_calls() {
        let mut gate = OcrRequestGate::default();
        assert_eq!(
            gate.run(|| Err("OCR unavailable".into()), Duration::ZERO)
                .unwrap_err(),
            "OCR unavailable"
        );
        let (request, _, _, _) = fake(vec![Ok(Some("recovered".into()))]);
        assert_eq!(
            gate.run(|| Ok(request), Duration::ZERO).unwrap(),
            "recovered"
        );
    }
    #[test]
    fn short_caller_budget_cancels_without_waiting_for_default_timeout() {
        let mut gate = OcrRequestGate::default();
        let (request, _, cancelled, _) = fake(vec![]);
        let started = Instant::now();
        assert!(gate.run(|| Ok(request), Duration::from_millis(20)).unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(500), "short read must not wait the legacy two-second budget");
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
    }

}
