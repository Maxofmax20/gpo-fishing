//! Vision provider abstraction.
//!
//! ```text
//! VisionProvider
//!     ↓
//! detect(frame)
//!     ↓
//! VisionObservation[]
//! ```
//!
//! Different implementations can be added later (including a trained
//! GPO-specific model) without touching the bot. What exists TODAY:
//!
//! * [`HeuristicBarProvider`] — REAL: delegates to the tested
//!   `core::vision` bar/fish/marker detectors and reports their confidence.
//! * [`NoopProvider`] — returns nothing (explicit off-switch / tests).
//!
//! General object detection (bite icons, bait rows, fruit popups, server
//! clock) has NO trained model in this codebase. Status:
//! ARCHITECTURE READY, MODEL TRAINING PENDING. Nothing here pretends
//! otherwise: unsupported targets simply yield no observations.

use serde::{Deserialize, Serialize};

use crate::core::types::Frame;
use crate::core::vision::{self, Palette};

/// Normalized detection box (0..1 relative to the input frame).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One visual observation. `confidence` is the underlying detector's own
/// score (rounded to 2dp) — never an invented probability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionObservation {
    pub label: String,
    pub confidence: f32,
    pub bbox: Option<RelBox>,
    pub detail: String,
}

pub trait VisionProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn detect(&self, frame: &Frame) -> Vec<VisionObservation>;
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// Real heuristic provider backed by `core::vision`.
///
/// Detects TODAY: `fishing_bar` (with score), `fish_present`,
/// `marker_present` (when a live reading exists). Everything else:
/// no observation (honest absence, not a zero-confidence guess).
#[derive(Debug, Clone)]
pub struct HeuristicBarProvider {
    palette: Palette,
}

impl HeuristicBarProvider {
    pub fn new(palette: Palette) -> Self {
        Self { palette }
    }
}

impl VisionProvider for HeuristicBarProvider {
    fn name(&self) -> &'static str {
        "heuristic_bar"
    }

    fn detect(&self, frame: &Frame) -> Vec<VisionObservation> {
        if frame.w == 0 || frame.h == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let conf = vision::confidence(frame, &self.palette);
        if conf.score > 0.0 {
            out.push(VisionObservation {
                label: "fishing_bar".to_string(),
                confidence: round2(conf.score),
                bbox: vision::find_bar(frame, &self.palette).map(|b| RelBox {
                    x: b.x0 as f32 / frame.w as f32,
                    y: b.y0 as f32 / frame.h as f32,
                    w: b.w() as f32 / frame.w as f32,
                    h: b.h() as f32 / frame.h as f32,
                }),
                detail: format!(
                    "histogram bar detector (bar {:.2}, fish {:.2}, marker {:.2})",
                    conf.bar, conf.fish, conf.marker
                ),
            });
        }
        if let Some(reading) = vision::read(frame, &self.palette) {
            if reading.fish_center > 0.0 {
                out.push(VisionObservation {
                    label: "fish_present".to_string(),
                    confidence: round2(conf.fish.clamp(0.0, 1.0)),
                    bbox: None,
                    detail: format!("dark fish span at {:.2}", reading.fish_center),
                });
            }
            if reading.marker_center > 0.0 {
                out.push(VisionObservation {
                    label: "marker_present".to_string(),
                    confidence: round2(conf.marker.clamp(0.0, 1.0)),
                    bbox: None,
                    detail: format!("white marker span at {:.2}", reading.marker_center),
                });
            }
        }
        out
    }
}

/// Explicit no-op provider: useful as an off-switch and in tests.
#[derive(Debug, Clone, Default)]
pub struct NoopProvider;

impl VisionProvider for NoopProvider {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn detect(&self, _frame: &Frame) -> Vec<VisionObservation> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar_frame() -> Frame {
        // Synthetic blue vertical bar on dark background (same construction
        // style as the vision unit tests, not a gameplay capture).
        let (w, h) = (60usize, 120usize);
        let mut rgba = vec![20u8; w * h * 4];
        for y in 10..110 {
            for x in 20..40 {
                let i = (y * w + x) * 4;
                rgba[i] = 85;
                rgba[i + 1] = 170;
                rgba[i + 2] = 255;
                rgba[i + 3] = 255;
            }
        }
        Frame::new(w, h, rgba)
    }

    #[test]
    fn heuristic_provider_reports_bar_with_bbox() {
        let p = HeuristicBarProvider::new(Palette::default());
        let obs = p.detect(&bar_frame());
        let bar = obs.iter().find(|o| o.label == "fishing_bar").expect("bar observation");
        assert!(bar.confidence > 0.0);
        assert!(bar.bbox.is_some());
        assert!(bar.confidence <= 1.0);
    }

    #[test]
    fn empty_frame_yields_no_observations_not_fake_zeros() {
        let p = HeuristicBarProvider::new(Palette::default());
        assert!(p.detect(&Frame::new(0, 0, Vec::new())).is_empty());
        // Plain noise: provider may report a weak bar or nothing — but must
        // never invent fish/marker presence without a reading.
        let noise = Frame::new(40, 40, vec![30u8; 40 * 40 * 4]);
        for o in p.detect(&noise) {
            assert_ne!(o.label, "fish_present");
            assert_ne!(o.label, "marker_present");
        }
    }

    #[test]
    fn noop_provider_is_empty() {
        assert!(NoopProvider.detect(&bar_frame()).is_empty());
    }
}
