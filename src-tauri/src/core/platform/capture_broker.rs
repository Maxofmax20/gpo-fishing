//! Prioritise controller capture; optional previews never queue ahead of it.
use super::{Capture, PlatformError, Result};
use crate::core::types::{Frame, PxRect};
use parking_lot::Mutex;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::{Duration, Instant};

pub struct PriorityCapture {
    source: Arc<dyn Capture>,
    gate: Mutex<()>,
    waiting: AtomicUsize,
    preview: Mutex<Option<(PxRect, Instant, Frame)>>,
}
impl PriorityCapture {
    pub fn new(source: Arc<dyn Capture>) -> Self {
        Self {source, gate:Mutex::new(()), waiting:AtomicUsize::new(0), preview:Mutex::new(None)}
    }
}
struct Waiting<'a>(&'a AtomicUsize);
impl Drop for Waiting<'_> {fn drop(&mut self) {self.0.fetch_sub(1,Ordering::SeqCst);}}
impl Capture for PriorityCapture {
    fn grab(&self, rect: PxRect) -> Result<Frame> {
        self.waiting.fetch_add(1,Ordering::SeqCst);
        let _waiting=Waiting(&self.waiting);
        let _gate=self.gate.lock();
        self.source.grab(rect)
    }
    fn grab_preview(&self, rect: PxRect) -> Result<Frame> {
        if let Some((region,at,frame))=self.preview.lock().as_ref() {
            if *region==rect && at.elapsed()<Duration::from_millis(100) {return Ok(frame.clone());}
        }
        if self.waiting.load(Ordering::SeqCst)>0 {
            return Err(PlatformError::Capture("Controller capture has priority".into()));
        }
        let _gate=self.gate.try_lock().ok_or_else(|| PlatformError::Capture("Capture busy".into()))?;
        if self.waiting.load(Ordering::SeqCst)>0 {
            return Err(PlatformError::Capture("Controller capture has priority".into()));
        }
        let frame=self.source.grab(rect)?;
        *self.preview.lock()=Some((rect,Instant::now(),frame.clone()));
        Ok(frame)
    }
    fn preview_age(&self, rect: PxRect) -> Option<Duration> {
        self.preview.lock().as_ref().filter(|(region,_,_)|*region==rect).map(|(_,at,_)|at.elapsed())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Count(AtomicUsize);
    impl Capture for Count {
        fn grab(&self, _:PxRect)->Result<Frame> {self.0.fetch_add(1,Ordering::SeqCst);Ok(Frame::new(1,1,vec![0;4]))}
    }
    #[test]
    fn previews_share_fresh_capture_but_controller_always_gets_new_pixels() {
        let source=Arc::new(Count(AtomicUsize::new(0)));
        let broker=PriorityCapture::new(source.clone());
        let rect=PxRect{x:0,y:0,w:1,h:1};
        broker.grab_preview(rect).unwrap();broker.grab_preview(rect).unwrap();
        assert_eq!(source.0.load(Ordering::SeqCst),1);
        broker.grab(rect).unwrap();
        assert_eq!(source.0.load(Ordering::SeqCst),2);
        assert!(broker.grab_preview(PxRect{x:5,..rect}).is_ok());
        assert_eq!(source.0.load(Ordering::SeqCst),3);
    }
    #[test]
    fn pending_controller_prevents_optional_capture() {
        let source=Arc::new(Count(AtomicUsize::new(0)));
        let broker=PriorityCapture::new(source.clone());
        broker.waiting.store(1,Ordering::SeqCst);
        assert!(broker.grab_preview(PxRect{x:0,y:0,w:1,h:1}).is_err());
        assert_eq!(source.0.load(Ordering::SeqCst),0);
    }
}
