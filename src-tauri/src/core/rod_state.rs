//! Conservative local rod/cast evidence. No input, network or catch accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RodEvidence {
    Equipped,
    Unequipped,
    Unknown,
}

/// A title or two distinct bait rows identify the menu; unrelated orange digits
/// and a missing catch banner cannot establish either equipment or a failed cast.
pub fn bait_menu_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("fishing baits")
        || ["legendary fish bait", "rare fish bait", "common fish bait"]
            .iter()
            .filter(|label| text.contains(**label))
            .count()
            >= 2
}

/// Outline geometry is learned ONLY while independent menu evidence identifies
/// the held item as the rod. No slot number, bright line or screen position is
/// itself allowed to identify an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

fn bright(p: (u8, u8, u8)) -> bool {
    p.0 >= 205 && p.1 >= 205 && p.2 >= 205
}

pub fn outlined(rect: SlotRect, pixel: &impl Fn(usize, usize) -> (u8, u8, u8)) -> bool {
    let SlotRect { x, y, w, h } = rect;
    if w < 8 || h < 8 {
        return false;
    }
    let horizontal = |yy| (0..w).filter(|dx| bright(pixel(x + dx, yy))).count() * 100 >= w * 82;
    let vertical = |xx| (0..h).filter(|dy| bright(pixel(xx, y + dy))).count() * 100 >= h * 82;
    if !horizontal(y) || !horizontal(y + h - 1) || !vertical(x) || !vertical(x + w - 1) {
        return false;
    }
    // Full white blocks, text underlines and filled UI surfaces are not outlines.
    let mut inside = 0;
    let mut count = 0;
    for dy in 3..h - 3 {
        for dx in 3..w - 3 {
            count += 1;
            if bright(pixel(x + dx, y + dy)) {
                inside += 1;
            }
        }
    }
    inside * 100 < count * 45
}

/// A positively selected different slot is negative rod evidence. Failure to
/// see the learned rectangle alone is never negative evidence (occlusion).
pub fn different_selected_slot(rod: SlotRect, selected: Option<SlotRect>) -> bool {
    selected.is_some_and(|other| {
        other.x + other.w <= rod.x
            || rod.x + rod.w <= other.x
            || other.y + other.h <= rod.y
            || rod.y + rod.h <= other.y
    })
}

pub fn find_outline(
    w: usize,
    h: usize,
    min_side: usize,
    max_side: usize,
    pixel: impl Fn(usize, usize) -> (u8, u8, u8),
) -> Option<SlotRect> {
    let mut candidates: Vec<SlotRect> = Vec::new();
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if !bright(pixel(x, y)) {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && bright(pixel(x, y)) {
                x += 1;
            }
            let width = x - start;
            if width < min_side || width > max_side {
                continue;
            }
            let low = width.saturating_sub(3).max(min_side);
            for height in low..=(width + 3).min(max_side) {
                if y + height > h {
                    continue;
                }
                let rect = SlotRect {
                    x: start,
                    y,
                    w: width,
                    h: height,
                };
                if outlined(rect, &pixel)
                    && !candidates
                        .iter()
                        .any(|old| old.x.abs_diff(start) < 4 && old.y.abs_diff(y) < 4)
                {
                    candidates.push(rect);
                }
            }
        }
    }
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

#[derive(Default)]
pub struct RodTracker {
    pub learned: Option<(SlotRect, usize, usize, char)>,
    pub recovery_attempts: u8,
    pub last_recovery: Option<std::time::Instant>,
    pub menu_region: Option<(f32, f32, f32, f32)>,
}

#[derive(Default)]
pub struct CastWatch {
    consecutive: u8,
}
impl CastWatch {
    /// The bait menu can remain visible after selecting bait and while a
    /// valid cast is underway. Only repeated, positive evidence that a
    /// *different toolbar slot* is selected can reject an attempted cast.
    pub fn observe(&mut self, elapsed_ms: u64, _menu_open: bool, rod: RodEvidence) -> bool {
        if elapsed_ms < 800 {
            self.consecutive = 0;
            return false;
        }
        if rod == RodEvidence::Unequipped {
            self.consecutive = self.consecutive.saturating_add(1);
        } else {
            self.consecutive = 0;
        }
        self.consecutive >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_requires_identity() {
        assert!(bait_menu_text("Fishing Baits"));
        assert!(bait_menu_text("Legendary Fish Bait x10 Rare Fish Bait x20"));
        assert!(!bait_menu_text("Rare Fish Bait x20"));
        assert!(!bait_menu_text("Caught Common Fish 0/100"));
    }
    #[test]
    fn absent_banner_and_unknown_state_do_not_reject_cast() {
        let mut w = CastWatch::default();
        for t in [800, 1300, 1800, 10000] {
            assert!(!w.observe(t, false, RodEvidence::Unknown));
        }
    }
    #[test]
    fn bait_menu_can_remain_open_during_valid_cast() {
        let mut w = CastWatch::default();
        for elapsed in [300, 800, 1300, 1800, 3000] {
            assert!(!w.observe(elapsed, true, RodEvidence::Equipped));
        }
        // Unknown toolbar state combined with a visible menu must not
        // trigger the rapid cast/recover loop reported in beta 1.
        for elapsed in [3500, 4000, 4500] {
            assert!(!w.observe(elapsed, true, RodEvidence::Unknown));
        }
    }
    #[test]
    fn failed_cast_evidence_must_be_consecutive() {
        let mut w = CastWatch::default();
        assert!(!w.observe(800, true, RodEvidence::Equipped));
        assert!(!w.observe(1300, false, RodEvidence::Unknown));
        assert!(!w.observe(1800, true, RodEvidence::Equipped));
    }
    #[test]
    fn affirmative_unequipped_rejects_cast() {
        let mut w = CastWatch::default();
        assert!(!w.observe(800, true, RodEvidence::Unequipped));
        assert!(w.observe(1300, true, RodEvidence::Unequipped));
    }
    fn pixels(rect: SlotRect, fill: bool) -> Vec<(u8, u8, u8)> {
        let mut p = vec![(20, 20, 20); 200 * 60];
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                if fill
                    || y == rect.y
                    || y == rect.y + rect.h - 1
                    || x == rect.x
                    || x == rect.x + rect.w - 1
                {
                    p[y * 200 + x] = (255, 255, 255);
                }
            }
        }
        p
    }
    #[test]
    fn learned_closed_outline_proves_held_rod_after_menu_closes() {
        let r = SlotRect {
            x: 30,
            y: 15,
            w: 32,
            h: 32,
        };
        let p = pixels(r, false);
        let found = find_outline(200, 60, 24, 40, |x, y| p[y * 200 + x]).unwrap();
        assert!(outlined(found, &|x, y| p[y * 200 + x]));
        // Removing it yields no positive proof, never affirmative unequipped.
        assert!(!outlined(found, &|_, _| (20, 20, 20)));
    }
    #[test]
    fn bright_lines_and_white_blocks_do_not_identify_slots() {
        let r = SlotRect {
            x: 30,
            y: 15,
            w: 32,
            h: 32,
        };
        let p = pixels(r, true);
        assert!(find_outline(200, 60, 24, 40, |x, y| p[y * 200 + x]).is_none());
        assert!(find_outline(200, 60, 24, 40, |_, y| if y == 15 {
            (255, 255, 255)
        } else {
            (0, 0, 0)
        })
        .is_none());
    }
    #[test]
    fn multiple_selected_outlines_are_ambiguous() {
        let r = SlotRect {
            x: 30,
            y: 15,
            w: 32,
            h: 32,
        };
        let p = pixels(r, false);
        let q = pixels(SlotRect { x: 90, ..r }, false);
        assert!(
            find_outline(200, 60, 24, 40, |x, y| if p[y * 200 + x].0 > 200 {
                p[y * 200 + x]
            } else {
                q[y * 200 + x]
            })
            .is_none()
        );
    }
    #[test]
    fn only_a_positive_different_selection_proves_unequipped() {
        let rod = SlotRect {
            x: 30,
            y: 15,
            w: 32,
            h: 32,
        };
        assert!(!different_selected_slot(rod, None));
        assert!(!different_selected_slot(rod, Some(rod)));
        assert!(different_selected_slot(
            rod,
            Some(SlotRect { x: 70, ..rod })
        ));
    }
}
