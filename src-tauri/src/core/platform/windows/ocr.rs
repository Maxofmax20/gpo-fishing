use parking_lot::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::core::platform::ocr_gate::{OcrPoll, OcrRequest, OcrRequestGate};
use crate::core::platform::{Ocr, PlatformError, Result};
use crate::core::types::Frame;

thread_local! {
    static APARTMENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn ensure_apartment() {
    APARTMENT.with(|done| {
        if !done.get() {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            done.set(true);
        }
    });
}

#[derive(Default)]
pub struct WindowsOcr {
    engine: OnceLock<Option<OcrEngine>>,
    requests: Mutex<OcrRequestGate>,
}

struct WindowsOcrRequest {
    poll: Box<dyn Fn() -> std::result::Result<OcrPoll, String> + Send>,
    cancel: Box<dyn Fn() + Send>,
}
impl OcrRequest for WindowsOcrRequest {
    fn poll(&self) -> std::result::Result<OcrPoll, String> {
        (self.poll)()
    }
    fn cancel(&self) {
        (self.cancel)();
    }
}

unsafe impl Send for WindowsOcr {}
unsafe impl Sync for WindowsOcr {}

impl WindowsOcr {
    pub fn new() -> Self {
        Self::default()
    }

    fn engine(&self) -> Option<&OcrEngine> {
        self.engine
            .get_or_init(|| {
                ensure_apartment();
                let engine = OcrEngine::TryCreateFromUserProfileLanguages()
                    .ok()
                    .or_else(|| {
                        let lang = Language::CreateLanguage(&HSTRING::from("en-US")).ok()?;
                        OcrEngine::TryCreateFromLanguage(&lang).ok()
                    });
                if engine.is_none() {
                    tracing::warn!("Windows OCR engine unavailable; fruit detection disabled");
                }
                engine
            })
            .as_ref()
    }
}

impl Ocr for WindowsOcr {
    fn available(&self) -> bool {
        self.engine().is_some()
    }

    fn read(&self, frame: &Frame) -> Result<String> {
        self.read_with_timeout(frame, Duration::from_secs(2))
    }

    fn read_with_timeout(&self, frame: &Frame, timeout: Duration) -> Result<String> {
        let started = Instant::now();
        if timeout.is_zero() {
            return Err(PlatformError::Ocr("OCR read budget expired".into()));
        }
        ensure_apartment();
        let mut requests = self
            .requests
            .try_lock()
            .ok_or_else(|| PlatformError::Ocr("OCR is busy with another frame".into()))?;
        let engine = self.engine().ok_or(PlatformError::OcrUnavailable)?;
        if frame.w < 8 || frame.h < 8 {
            return Ok(String::new());
        }
        let expected_len = frame
            .w
            .checked_mul(frame.h)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected_len != Some(frame.rgba.len())
            || frame.w > i32::MAX as usize
            || frame.h > i32::MAX as usize
        {
            return Err(PlatformError::Ocr(
                "Invalid OCR frame dimensions or buffer length".into(),
            ));
        }
        let mut bgra = frame.rgba.clone();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let err = |e: windows::core::Error| PlatformError::Ocr(e.message());
        let writer = DataWriter::new().map_err(err)?;
        writer.WriteBytes(&bgra).map_err(err)?;
        let buffer = writer.DetachBuffer().map_err(err)?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.w as i32,
            frame.h as i32,
            BitmapAlphaMode::Ignore,
        )
        .map_err(err)?;
        let remaining = timeout.checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| PlatformError::Ocr("OCR read budget expired during frame preparation".into()))?;
        requests
            .run(
                || {
                    let operation = engine
                        .RecognizeAsync(&bitmap)
                        .map_err(|error| error.message())?;
                    let cancel_operation = operation.clone();
                    Ok(Box::new(WindowsOcrRequest {
                        poll: Box::new(move || {
                            // Keep the source bitmap alive while WinRT may still be using it.
                            let _bitmap = &bitmap;
                            // Windows.Foundation.AsyncStatus: Started=0, Completed=1,
                            // Canceled=2, Error=3. Status is polled; GetResults is never
                            // called while Started, so a wedged OCR cannot block Stop forever.
                            match operation.Status().map_err(|error| error.message())?.0 {
                                0 => Ok(OcrPoll::Pending),
                                1 => Ok(OcrPoll::Finished(
                                    operation
                                        .GetResults()
                                        .map_err(|error| error.message())
                                        .and_then(|result| result_text(&result)),
                                )),
                                2 => Ok(OcrPoll::Finished(Err("OCR request was cancelled".into()))),
                                3 => {
                                    Ok(OcrPoll::Finished(Err("Windows OCR request failed".into())))
                                }
                                _ => Err("Windows OCR returned an unknown operation status".into()),
                            }
                        }),
                        cancel: Box::new(move || {
                            let _ = cancel_operation.Cancel();
                        }),
                    }))
                },
                remaining,
            )
            .map_err(PlatformError::Ocr)
    }
}

fn result_text(result: &windows::Media::Ocr::OcrResult) -> std::result::Result<String, String> {
    let mut lines_text = Vec::new();
    if let Ok(lines) = result.Lines() {
        for line in lines {
            if let Ok(t) = line.Text() {
                let s = t.to_string();
                if !s.trim().is_empty() {
                    lines_text.push(s);
                }
            }
        }
    }
    if !lines_text.is_empty() {
        Ok(lines_text.join("\n"))
    } else {
        let text = result.Text().map_err(|error| error.message())?;
        Ok(text.to_string())
    }
}
