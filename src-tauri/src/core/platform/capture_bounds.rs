//! Checked pixel geometry shared by the native capture backends.
//! Negative desktop origins are valid; empty, overflowing or oversized buffers are not.

const MAX_CAPTURE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureLayout {
    pub width: usize,
    pub height: usize,
    pub row_bytes: usize,
    pub byte_len: usize,
}

impl CaptureLayout {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Result<Self, &'static str> {
        if width <= 0 || height <= 0 {
            return Err("empty capture rectangle");
        }
        x.checked_add(width)
            .ok_or("capture x coordinate overflow")?;
        y.checked_add(height)
            .ok_or("capture y coordinate overflow")?;
        let width = usize::try_from(width).map_err(|_| "capture width overflow")?;
        let height = usize::try_from(height).map_err(|_| "capture height overflow")?;
        let row_bytes = width.checked_mul(4).ok_or("capture row overflow")?;
        let byte_len = row_bytes
            .checked_mul(height)
            .ok_or("capture buffer overflow")?;
        if byte_len > MAX_CAPTURE_BYTES || byte_len > isize::MAX as usize {
            return Err("capture buffer exceeds 256 MiB safety limit");
        }
        Ok(Self {
            width,
            height,
            row_bytes,
            byte_len,
        })
    }

    pub fn allocate(self) -> Result<Vec<u8>, &'static str> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.byte_len)
            .map_err(|_| "capture allocation failed")?;
        bytes.resize(self.byte_len, 0);
        Ok(bytes)
    }
}

/// Returns local coordinates only when the WHOLE rectangle fits the output.
/// i64 subtraction also permits monitors left of or above the primary monitor.
pub fn contained_offset(
    rect: (i32, i32, i32, i32),
    origin: (i32, i32),
    size: (i32, i32),
) -> Option<(usize, usize)> {
    let (x, y, w, h) = rect;
    if w <= 0 || h <= 0 || size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    let local_x = i64::from(x) - i64::from(origin.0);
    let local_y = i64::from(y) - i64::from(origin.1);
    if local_x < 0
        || local_y < 0
        || local_x.checked_add(i64::from(w))? > i64::from(size.0)
        || local_y.checked_add(i64::from(h))? > i64::from(size.1)
    {
        return None;
    }
    Some((
        usize::try_from(local_x).ok()?,
        usize::try_from(local_y).ok()?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedRows {
    pub first_byte: usize,
    pub pitch: usize,
    pub row_bytes: usize,
    pub rows: usize,
}

/// Validate all mapped memory offsets before any pointer arithmetic or slice construction.
pub fn mapped_rows(
    layout: CaptureLayout,
    local: (usize, usize),
    texture_size: (i32, i32),
    pitch: usize,
) -> Result<MappedRows, &'static str> {
    let texture_width = usize::try_from(texture_size.0).map_err(|_| "invalid texture width")?;
    let texture_height = usize::try_from(texture_size.1).map_err(|_| "invalid texture height")?;
    if texture_width == 0 || texture_height == 0 || layout.width == 0 || layout.height == 0 {
        return Err("empty mapped texture");
    }
    let min_pitch = texture_width.checked_mul(4).ok_or("texture row overflow")?;
    if pitch < min_pitch {
        return Err("mapped row pitch is smaller than texture width");
    }
    if local
        .0
        .checked_add(layout.width)
        .ok_or("mapped x overflow")?
        > texture_width
        || local
            .1
            .checked_add(layout.height)
            .ok_or("mapped y overflow")?
            > texture_height
    {
        return Err("capture rectangle crosses mapped texture bounds");
    }
    if layout.width.checked_mul(4) != Some(layout.row_bytes)
        || layout.row_bytes.checked_mul(layout.height) != Some(layout.byte_len)
        || layout.byte_len > MAX_CAPTURE_BYTES
    {
        return Err("inconsistent capture layout");
    }
    let mapped_bytes = pitch
        .checked_mul(texture_height)
        .ok_or("mapped texture length overflow")?;
    let first_byte = local
        .1
        .checked_mul(pitch)
        .and_then(|row| {
            local
                .0
                .checked_mul(4)
                .and_then(|column| row.checked_add(column))
        })
        .ok_or("mapped start offset overflow")?;
    let last_end = (layout.height - 1)
        .checked_mul(pitch)
        .and_then(|rows| first_byte.checked_add(rows))
        .and_then(|start| start.checked_add(layout.row_bytes))
        .ok_or("mapped end offset overflow")?;
    if last_end > mapped_bytes || mapped_bytes > isize::MAX as usize {
        return Err("mapped texture offset exceeds addressable bounds");
    }
    Ok(MappedRows {
        first_byte,
        pitch,
        row_bytes: layout.row_bytes,
        rows: layout.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_monitor_coordinates_are_valid() {
        let layout = CaptureLayout::new(-1920, -1080, 1920, 1080).unwrap();
        assert_eq!(layout.byte_len, 1920 * 1080 * 4);
        assert_eq!(
            contained_offset((-1900, -1000, 100, 200), (-1920, -1080), (1920, 1080)),
            Some((20, 80))
        );
    }

    #[test]
    fn rejects_empty_overflowing_and_oversized_rectangles() {
        for rect in [
            (0, 0, 0, 10),
            (0, 0, 10, -1),
            (i32::MAX, 0, 2, 2),
            (0, i32::MAX, 1, 1),
            (0, 0, i32::MAX, i32::MAX),
            (0, 0, 16384, 16384),
        ] {
            assert!(
                CaptureLayout::new(rect.0, rect.1, rect.2, rect.3).is_err(),
                "{rect:?}"
            );
        }
        assert!(CaptureLayout::new(0, 0, 7680, 4320).is_ok());
    }

    #[test]
    fn whole_rectangle_must_fit_not_only_top_left() {
        assert_eq!(
            contained_offset((1910, 20, 20, 10), (0, 0), (1920, 1080)),
            None
        );
        assert_eq!(
            contained_offset((20, 1070, 10, 20), (0, 0), (1920, 1080)),
            None
        );
        assert_eq!(
            contained_offset((-1, 0, 10, 10), (0, 0), (1920, 1080)),
            None
        );
        assert_eq!(
            contained_offset((1910, 1070, 10, 10), (0, 0), (1920, 1080)),
            Some((1910, 1070))
        );
        assert_eq!(
            contained_offset((i32::MAX - 1, 0, 2, 1), (i32::MIN, 0), (i32::MAX, 1)),
            None
        );
    }

    #[test]
    fn padded_row_pitch_and_last_pixel_are_checked() {
        let layout = CaptureLayout::new(2, 1, 2, 2).unwrap();
        assert_eq!(
            mapped_rows(layout, (2, 1), (4, 3), 24).unwrap(),
            MappedRows {
                first_byte: 32,
                pitch: 24,
                row_bytes: 8,
                rows: 2
            }
        );
        assert!(mapped_rows(layout, (3, 1), (4, 3), 24).is_err());
        assert!(mapped_rows(layout, (2, 2), (4, 3), 24).is_err());
        assert!(mapped_rows(layout, (2, 1), (4, 3), 15).is_err());
    }

    #[test]
    fn mapped_offsets_reject_integer_overflow() {
        let layout = CaptureLayout::new(0, 0, 1, 1).unwrap();
        assert!(mapped_rows(layout, (0, 0), (1, 2), usize::MAX).is_err());
        assert!(mapped_rows(layout, (usize::MAX, 0), (2, 2), 8).is_err());
        assert!(mapped_rows(layout, (0, usize::MAX), (2, 2), 8).is_err());
        assert!(mapped_rows(layout, (0, 0), (0, 2), 8).is_err());
    }

    #[test]
    fn inconsistent_layout_cannot_create_an_oversized_source_slice() {
        let layout = CaptureLayout {
            width: 1,
            height: 1,
            row_bytes: 8,
            byte_len: 8,
        };
        assert!(mapped_rows(layout, (0, 0), (4, 4), 16).is_err());
    }

    #[test]
    fn allocation_matches_validated_layout() {
        let layout = CaptureLayout::new(0, 0, 7, 3).unwrap();
        let bytes = layout.allocate().unwrap();
        assert_eq!(bytes.len(), 84);
        assert!(bytes.iter().all(|byte| *byte == 0));
    }
}
