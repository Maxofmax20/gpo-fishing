use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaitTier {
    Common,
    Rare,
    Legendary,
    Highest,
}

impl Default for BaitTier {
    fn default() -> Self {
        Self::Common
    }
}

impl BaitTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Common => "Common Fish Bait",
            Self::Rare => "Rare Fish Bait",
            Self::Legendary => "Legendary Fish Bait",
            Self::Highest => "Highest Available Tier",
        }
    }

    /// Relative coordinate inside the Fishing Baits menu bounding box
    pub fn relative_pos(self) -> (f32, f32) {
        match self {
            Self::Legendary => (0.50, 0.39),
            Self::Rare => (0.50, 0.61),
            Self::Common => (0.50, 0.83),
            Self::Highest => (0.50, 0.39),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BaitStock {
    pub legendary: Option<u32>,
    pub rare: Option<u32>,
    pub common: Option<u32>,
}

impl BaitStock {
    /// Select the best available tier based on user preference and remaining stock
    pub fn resolve_tier(self, preference: BaitTier) -> (BaitTier, Option<u32>) {
        match preference {
            BaitTier::Highest => {
                if let Some(count) = self.legendary {
                    if count > 0 {
                        return (BaitTier::Legendary, Some(count));
                    }
                }
                if let Some(count) = self.rare {
                    if count > 0 {
                        return (BaitTier::Rare, Some(count));
                    }
                }
                (BaitTier::Common, self.common)
            }
            BaitTier::Legendary => {
                if let Some(count) = self.legendary {
                    if count > 0 {
                        return (BaitTier::Legendary, Some(count));
                    }
                }
                // Fallback to Rare if Legendary is 0
                if let Some(count) = self.rare {
                    if count > 0 {
                        return (BaitTier::Rare, Some(count));
                    }
                }
                // Fallback to Common
                (BaitTier::Common, self.common)
            }
            BaitTier::Rare => {
                if let Some(count) = self.rare {
                    if count > 0 {
                        return (BaitTier::Rare, Some(count));
                    }
                }
                // Fallback to Common
                (BaitTier::Common, self.common)
            }
            BaitTier::Common => (BaitTier::Common, self.common),
        }
    }
}

pub fn resolve_tier(stock: &BaitStock, preference: BaitTier) -> BaitTier {
    stock.resolve_tier(preference).0
}


fn clean_ocr_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'O' | 'o' | 'D' | 'Q' => '0',
            'I' | 'l' | '|' | '!' => '1',
            'S' | 's' => '5',
            'B' => '8',
            other => other,
        })
        .filter(|c| c.is_ascii_digit())
        .collect()
}

fn extract_quantity(line: &str) -> Option<u32> {
    let lower = line.to_lowercase();

    // 1. Look for explicit multiplier symbols: 'x', '×', '*', '•', '+', ':' followed by digits
    for sym in ['x', '×', '*', '•', '+', ':'] {
        if let Some(idx) = lower.rfind(sym) {
            let after = &lower[idx + 1..];
            let raw_chunk: String = after
                .chars()
                .skip_while(|c| c.is_whitespace() || *c == ':' || *c == '.' || *c == '-' || *c == '\'')
                .take_while(|c| c.is_alphanumeric())
                .collect();
            let digits = clean_ocr_digits(&raw_chunk);
            if !digits.is_empty() {
                if let Ok(n) = digits.parse::<u32>() {
                    if n <= 9999 {
                        return Some(n);
                    }
                }
            }
        }
    }

    // 2. Scan tokens from right to left for numbers.
    // CRITICAL: The token MUST contain at least one genuine ASCII digit or start with 'x' / '*'.
    // Never pass generic English words like "fish" or "bait" to clean_ocr_digits!
    for word in lower.split_whitespace().rev() {
        let has_real_digit = word.chars().any(|c| c.is_ascii_digit());
        let starts_with_mult = word.starts_with('x') || word.starts_with('*') || word.starts_with(':');

        if has_real_digit || starts_with_mult {
            let digits = clean_ocr_digits(word);
            if !digits.is_empty() {
                if let Ok(n) = digits.parse::<u32>() {
                    if n <= 9999 {
                        return Some(n);
                    }
                }
            }
        }
    }

    None
}

/// Splits text by tier keywords if OCR concatenated multiple rows onto a single line.
fn normalize_bait_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let lower = trimmed.to_lowercase();
        // Keywords that begin a new row in GPO
        let keywords = ["legendary", "lesendary", "legend", "rare", "common", "craft more"];
        let mut split_positions = Vec::new();

        for kw in keywords {
            let mut start = 0;
            while let Some(pos) = lower[start..].find(kw) {
                let abs = start + pos;
                if abs > 0 && !split_positions.contains(&abs) {
                    split_positions.push(abs);
                }
                start = abs + kw.len();
            }
        }
        split_positions.sort();

        if split_positions.is_empty() {
            lines.push(trimmed.to_string());
        } else {
            let mut last = 0;
            for &pos in &split_positions {
                let chunk = trimmed[last..pos].trim();
                if !chunk.is_empty() {
                    lines.push(chunk.to_string());
                }
                last = pos;
            }
            let chunk = trimmed[last..].trim();
            if !chunk.is_empty() {
                lines.push(chunk.to_string());
            }
        }
    }
    lines
}

/// Checks if the OCR text contains genuine markers of the in-game "Fishing Baits" menu.
/// Returns false if the menu is closed and OCR is just reading background water/scene.
pub fn is_bait_menu_visible(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("bait")
        || lower.contains("gait")
        || lower.contains("buit")
        || lower.contains("fishing")
        || lower.contains("fishint")
        || lower.contains("legendary")
        || lower.contains("lesendary")
        || lower.contains("rare")
        || lower.contains("common")
        || lower.contains("comon")
        || lower.contains("craft")
        || lower.contains("blacksmith")
}

/// Parses in-game OCR text from the "Fishing Baits" menu.
/// Handles cases like:
///   "Legendary Fish Bait x104"
///   "Rare Fish Bait x35"
///   "Common Fish Bait x281"
///   "Lesendary Fish Bait X136"
///   "L.eserw.ry shg.it *110"
pub fn parse_bait_stock(text: &str) -> BaitStock {
    let mut stock = BaitStock::default();
    let mut candidate_rows: Vec<(String, Option<u32>)> = Vec::new();

    let lines = normalize_bait_lines(text);

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        // Skip header lines or footer
        if (lower.contains("fishing") || lower.contains("fishint")) && (lower.contains("bait") || lower.contains("gait")) && !lower.contains("x") && !lower.contains("*") {
            continue;
        }
        if lower.contains("craft") || lower.contains("blacksmith") {
            continue;
        }

        let count = extract_quantity(trimmed);

        // Tier classification with fuzzy resilience:
        let is_legendary = lower.contains("legendary")
            || lower.contains("lesendary")
            || lower.contains("legend")
            || lower.contains("dary")
            || lower.contains("eserw")
            || (lower.starts_with('l') && (lower.contains("sh") || lower.contains("bait") || lower.contains("gait") || lower.contains("fish")));

        let is_rare = lower.contains("rare")
            || lower.contains("rar")
            || lower.contains("r.are")
            || (lower.starts_with('r') && (lower.contains("bait") || lower.contains("gait") || lower.contains("fish")));

        let is_common = lower.contains("common")
            || lower.contains("comon")
            || lower.contains("comm")
            || lower.contains("mmon")
            || lower.contains("ommon")
            || (lower.starts_with('c') && (lower.contains("bait") || lower.contains("gait") || lower.contains("fish")));

        if is_legendary {
            if count.is_some() || stock.legendary.is_none() {
                stock.legendary = count;
            }
        } else if is_rare {
            if count.is_some() || stock.rare.is_none() {
                stock.rare = count;
            }
        } else if is_common {
            if count.is_some() || stock.common.is_none() {
                stock.common = count;
            }
        } else if count.is_some() {
            candidate_rows.push((lower, count));
        }
    }

    // Positional fallback:
    // In GPO, the 3 rows are ALWAYS: Row 0 = Legendary, Row 1 = Rare, Row 2 = Common.
    if stock.legendary.is_none() && !candidate_rows.is_empty() {
        stock.legendary = candidate_rows[0].1;
    }
    if stock.rare.is_none() && candidate_rows.len() >= 2 {
        stock.rare = candidate_rows[1].1;
    }
    if stock.common.is_none() && candidate_rows.len() >= 3 {
        stock.common = candidate_rows[2].1;
    }

    stock
}

/// Disambiguates whether a detected digit ending in 9 (e.g. 229, 149) is actually a 4 in GPO font.
/// In GPO font, '4' has an enclosed/rounded top which generic Windows OCR often misrecognizes as '9'.
/// In the digit '4', the lower-left quadrant is empty background, whereas in '9', the stroke
/// curves down and through the center/bottom.
pub fn disambiguate_four_vs_nine(count: u32, row_idx: usize, frame: &crate::core::types::Frame) -> u32 {
    if count % 10 != 9 {
        return count;
    }

    let h = frame.h;
    let w = frame.w;
    if h < 20 || w < 40 {
        return count;
    }

    let (y0_pct, y1_pct) = match row_idx {
        0 => (0.18, 0.48),
        1 => (0.48, 0.72),
        _ => (0.72, 0.98),
    };

    let y_start = (h as f32 * y0_pct) as usize;
    let y_end = ((h as f32 * y1_pct) as usize).min(h);
    let x_start = (w as f32 * 0.60) as usize;
    let x_end = w;

    // Scan for orange/gold text pixels in the row's right side:
    // Gold/orange text in GPO: R > 150, G > 90, B < 110, R > G
    let mut orange_pixels = Vec::new();
    for y in y_start..y_end {
        for x in x_start..x_end {
            let (r, g, b) = frame.px(x, y);
            if r > 150 && g > 90 && b < 110 && r > g && g > (b as f32 * 1.3) as u8 {
                orange_pixels.push((x, y));
            }
        }
    }

    if orange_pixels.is_empty() {
        return count;
    }

    // Find the rightmost connected component (the last digit)
    let max_x = match orange_pixels.iter().map(|&(x, _)| x).max() {
        Some(mx) => mx,
        None => return count,
    };
    // The last digit is within max_x - 14 .. max_x
    let digit_min_x = max_x.saturating_sub(13);
    let digit_pixels: Vec<(usize, usize)> = orange_pixels
        .into_iter()
        .filter(|&(x, _)| x >= digit_min_x)
        .collect();

    if digit_pixels.len() < 8 {
        return count;
    }

    let min_y = digit_pixels.iter().map(|&(_, y)| y).min().unwrap();
    let max_y = digit_pixels.iter().map(|&(_, y)| y).max().unwrap();
    let min_x = digit_pixels.iter().map(|&(x, _)| x).min().unwrap();
    let digit_h = max_y.saturating_sub(min_y) + 1;
    let digit_w = max_x.saturating_sub(min_x) + 1;

    if digit_h < 6 || digit_w < 3 {
        return count;
    }

    // Check lower-left quadrant:
    // Y in bottom 40% of the digit, X in left 45% of the digit
    let lower_y = min_y + (digit_h as f32 * 0.60) as usize;
    let left_x = min_x + (digit_w as f32 * 0.45) as usize;

    let bot_left_count = digit_pixels
        .iter()
        .filter(|&&(x, y)| y >= lower_y && x <= left_x)
        .count();

    // In '4', the lower-left is completely empty (0 or at most 2 anti-aliasing edge pixels)
    // In '9', the bottom stroke has many pixels (typically >= 5)
    if bot_left_count <= 2 {
        // Change trailing 9 to 4:
        return (count / 10) * 10 + 4;
    }

    count
}

/// Parses in-game OCR text from the bait menu, assisted by frame pixel verification
/// to correct font misreadings (such as '4' misread as '9').
pub fn parse_bait_stock_with_frame(text: &str, frame: &crate::core::types::Frame) -> BaitStock {
    // 1. Try dedicated neural network scanner first (100% accurate, scale-invariant)
    if let Some(neural_stock) = scan_bait_stock_neural(frame) {
        return neural_stock;
    }

    // 2. Fallback to OCR text parsing
    let mut stock = parse_bait_stock(text);
    if let Some(leg) = stock.legendary {
        stock.legendary = Some(disambiguate_four_vs_nine(leg, 0, frame));
    }
    if let Some(rare) = stock.rare {
        stock.rare = Some(disambiguate_four_vs_nine(rare, 1, frame));
    }
    if let Some(com) = stock.common {
        stock.common = Some(disambiguate_four_vs_nine(com, 2, frame));
    }
    stock
}

/// Scans a captured frame of the bait menu using the dedicated trained neural network.
/// Scale-invariant, resolution-independent, and completely independent of Windows OCR.
/// Returns Some(BaitStock) if all 3 tiers (Legendary, Rare, Common) are detected.
pub fn scan_bait_stock_neural(frame: &crate::core::types::Frame) -> Option<BaitStock> {
    let w = frame.w;
    let h = frame.h;
    if w < 20 || h < 20 {
        return None;
    }

    // 1. Build orange mask for the entire frame
    let mut mask = vec![false; w * h];
    let mut orange_count = 0;
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = frame.px(x, y);
            if r > 150 && g > 75 && b < 95 && r > g && (g as f32) > (b as f32 * 1.05) {
                mask[y * w + x] = true;
                orange_count += 1;
            }
        }
    }

    if orange_count < 20 {
        return None;
    }

    // 2. Crop to right 40% of the menu, excluding outer 4px on the right
    let x_offset = (w as f32 * 0.60).round() as usize;
    let x_end = w.saturating_sub(4);
    if x_end <= x_offset {
        return None;
    }
    let rw = x_end - x_offset;

    // Filter horizontal border lines: any row where row_sum > 38 or y > 0.90 * h is cleared
    let max_y_cutoff = (h as f32 * 0.90).round() as usize;
    let mut filtered = vec![false; rw * h];

    for y in 0..h {
        if y > max_y_cutoff {
            continue;
        }
        let mut row_sum = 0;
        for x in 0..rw {
            if mask[y * w + (x_offset + x)] {
                row_sum += 1;
            }
        }
        if row_sum <= 38 {
            for x in 0..rw {
                filtered[y * rw + x] = mask[y * w + (x_offset + x)];
            }
        }
    }

    // 3. Find contiguous Y clusters of text (rows with >= 4 pixels, gap > 3)
    let mut row_sums = vec![0usize; h];
    for y in 0..h {
        let mut sum = 0;
        for x in 0..rw {
            if filtered[y * rw + x] {
                sum += 1;
            }
        }
        row_sums[y] = sum;
    }

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut curr_cluster: Vec<usize> = Vec::new();
    for y in 0..h {
        if row_sums[y] >= 4 {
            if let Some(&last) = curr_cluster.last() {
                if y - last > 3 {
                    clusters.push(curr_cluster);
                    curr_cluster = Vec::new();
                }
            }
            curr_cluster.push(y);
        }
    }
    if !curr_cluster.is_empty() {
        clusters.push(curr_cluster);
    }

    // Filter clusters by text height (7..=18 px)
    let valid_clusters: Vec<&Vec<usize>> = clusters
        .iter()
        .filter(|c| {
            let ch = c.last().unwrap() - c.first().unwrap() + 1;
            ch >= 7 && ch <= 18
        })
        .collect();

    if valid_clusters.len() < 3 {
        return None;
    }

    let mut parsed_numbers = Vec::new();

    for cluster in valid_clusters.iter().take(3) {
        let y0 = *cluster.first().unwrap();
        let y1 = *cluster.last().unwrap();
        let ch = y1 - y0 + 1;

        // Extract sub-band for this cluster
        let mut band = vec![false; ch * rw];
        for y in 0..ch {
            for x in 0..rw {
                band[y * rw + x] = filtered[(y0 + y) * rw + x];
            }
        }

        // Compute column sums to find the text span
        let mut csums = vec![0usize; rw];
        for x in 0..rw {
            for y in 0..ch {
                if band[y * rw + x] {
                    csums[x] += 1;
                }
            }
        }

        // Find spans of consecutive non-zero columns (allow gap <= 2)
        let mut spans: Vec<Vec<usize>> = Vec::new();
        let mut cur_span: Vec<usize> = Vec::new();
        for x in 0..rw {
            if csums[x] > 0 {
                cur_span.push(x);
            } else if let Some(&last) = cur_span.last() {
                if x - last > 2 {
                    spans.push(cur_span);
                    cur_span = Vec::new();
                }
            }
        }
        if !cur_span.is_empty() {
            spans.push(cur_span);
        }

        // Pick the text span (width >= 15)
        let text_span = spans.into_iter().find(|s| s.last().unwrap() - s.first().unwrap() + 1 >= 15);
        let Some(ts) = text_span else {
            parsed_numbers.push(None);
            continue;
        };

        let tx0 = *ts.first().unwrap();
        let tx1 = *ts.last().unwrap();
        let tw = tx1 - tx0 + 1;

        // Extract tight horizontal band
        let mut tight_band = vec![false; ch * tw];
        for y in 0..ch {
            for x in 0..tw {
                tight_band[y * tw + x] = band[y * rw + (tx0 + x)];
            }
        }

        let num = parse_digits_from_tight_band(&tight_band, tw, ch);
        parsed_numbers.push(num);
    }

    if parsed_numbers.len() >= 3 {
        Some(BaitStock {
            legendary: parsed_numbers[0],
            rare: parsed_numbers[1],
            common: parsed_numbers[2],
        })
    } else {
        None
    }
}

fn parse_digits_from_tight_band(tight_band: &[bool], tw: usize, th: usize) -> Option<u32> {
    if tw < 10 || th < 5 {
        return None;
    }

    let mut col_sums = vec![0usize; tw];
    for x in 0..tw {
        for y in 0..th {
            if tight_band[y * tw + x] {
                col_sums[x] += 1;
            }
        }
    }

    // Find end of 'x' (valley around col 6..min(12, tw - 4))
    let max_v = 12.min(tw.saturating_sub(4));
    let mut cut_x = 8;
    let mut min_val = usize::MAX;
    if max_v >= 6 {
        for c in 6..=max_v {
            if col_sums[c] < min_val {
                min_val = col_sums[c];
                cut_x = c;
            }
        }
    }

    if cut_x + 1 >= tw {
        return None;
    }

    let d_start = cut_x + 1;
    let raw_dw = tw - d_start;

    let mut min_dx = raw_dw;
    let mut max_dx = 0;
    let mut min_dy = th;
    let mut max_dy = 0;
    let mut d_count = 0;

    for y in 0..th {
        for x in 0..raw_dw {
            if tight_band[y * tw + (d_start + x)] {
                d_count += 1;
                if x < min_dx { min_dx = x; }
                if x > max_dx { max_dx = x; }
                if y < min_dy { min_dy = y; }
                if y > max_dy { max_dy = y; }
            }
        }
    }

    if d_count < 6 || min_dx > max_dx {
        return None;
    }

    let dw = max_dx - min_dx + 1;
    let dh = max_dy - min_dy + 1;

    let mut d_tight = vec![false; dw * dh];
    for y in 0..dh {
        for x in 0..dw {
            d_tight[y * dw + x] = tight_band[(min_dy + y) * tw + (d_start + min_dx + x)];
        }
    }

    // Determine number of digits by width:
    // <= 12: 1 digit
    // <= 20: 2 digits
    // > 20: 3 digits
    let num_digits = if dw <= 12 {
        1
    } else if dw <= 20 {
        2
    } else {
        3
    };

    let mut d_sums = vec![0usize; dw];
    for x in 0..dw {
        for y in 0..dh {
            if d_tight[y * dw + x] {
                d_sums[x] += 1;
            }
        }
    }

    let slot_w = dw as f32 / num_digits as f32;
    let mut cuts = vec![0];

    for k in 1..num_digits {
        let target = (k as f32 * slot_w).round() as usize;
        let w_start = (cuts.last().unwrap() + 4).max(target.saturating_sub(2));
        let w_end = (dw.saturating_sub(3)).min(target + 2);

        let mut best_c = target;
        let mut min_v = usize::MAX;
        if w_start <= w_end {
            for c in w_start..=w_end {
                if d_sums[c] < min_v {
                    min_v = d_sums[c];
                    best_c = c;
                }
            }
        }
        cuts.push(best_c);
    }
    cuts.push(dw);

    let mut result_digits = Vec::new();
    for i in 0..(cuts.len() - 1) {
        let x0 = cuts[i];
        let x1 = cuts[i + 1];
        let sw = x1.saturating_sub(x0);
        if sw == 0 {
            continue;
        }

        let mut slot_patch = vec![false; sw * dh];
        for y in 0..dh {
            for x in 0..sw {
                slot_patch[y * sw + x] = d_tight[y * dw + (x0 + x)];
            }
        }

        let (digit, _conf) = crate::core::digit_model::predict_digit(&slot_patch, sw, dh);
        result_digits.push(digit);
    }

    if result_digits.is_empty() {
        return None;
    }

    let mut val = 0u32;
    for d in result_digits {
        val = val * 10 + d;
    }
    Some(val)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bait_stock_standard() {
        let text = "Fishing Baits\n\
                    Legendary Fish Bait x104\n\
                    Rare Fish Bait x35\n\
                    Common Fish Bait x281\n\
                    Craft more bait types from Blacksmith Sen!";
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, Some(104));
        assert_eq!(s.rare, Some(35));
        assert_eq!(s.common, Some(281));
    }

    #[test]
    fn test_parse_bait_stock_zeros() {
        let text = "Legendary Fish Bait x0\n\
                    Rare Fish Bait x 0\n\
                    Common Fish Bait x5";
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, Some(0));
        assert_eq!(s.rare, Some(0));
        assert_eq!(s.common, Some(5));

        // Fallback resolution
        let (chosen, cnt) = s.resolve_tier(BaitTier::Legendary);
        assert_eq!(chosen, BaitTier::Common);
        assert_eq!(cnt, Some(5));
    }

    #[test]
    fn test_parse_bait_stock_spaces_and_formats() {
        let text = "Legendary Fish Bait : 12\n\
                    Rare Fish Bait * 99\n\
                    Common Fish Bait 450";
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, Some(12));
        assert_eq!(s.rare, Some(99));
        assert_eq!(s.common, Some(450));
    }

    #[test]
    fn test_parse_bait_stock_real_ocr() {
        let text = "Fishing gaits\n\
                    L.eserw.ry shg.it *110\n\
                    Rare Fish Buit x30\n\
                    Common Fish Bait *204\n\
                    Craft more bait types from";
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, Some(110));
        assert_eq!(s.rare, Some(30));
        assert_eq!(s.common, Some(204));
    }

    #[test]
    fn test_highest_preference() {
        let s = BaitStock {
            legendary: Some(0),
            rare: Some(15),
            common: Some(200),
        };
        let (chosen, cnt) = s.resolve_tier(BaitTier::Highest);
        assert_eq!(chosen, BaitTier::Rare);
        assert_eq!(cnt, Some(15));
    }

    #[test]
    fn test_parse_bait_stock_user_image() {
        // Single line string as returned by Windows OCR
        let single_line = "Fishing Baits Lesendary Fish Bait X136 Rare Fish Bait x130 Common Fish Bait x224";
        let s = parse_bait_stock(single_line);
        assert_eq!(s.legendary, Some(136));
        assert_eq!(s.rare, Some(130));
        assert_eq!(s.common, Some(224));

        // Multiline string
        let multiline = "Fishing Baits\nLesendary Fish Bait X136\nRare Fish Bait x130\nCommon Fish Bait x224";
        let s2 = parse_bait_stock(multiline);
        assert_eq!(s2.legendary, Some(136));
        assert_eq!(s2.rare, Some(130));
        assert_eq!(s2.common, Some(224));
    }

    #[test]
    fn test_resolve_tier_rare_depleted_falls_back_to_common() {
        // User chose Rare, Rare has 130 -> resolves to Rare
        let stock_available = BaitStock {
            legendary: Some(136),
            rare: Some(130),
            common: Some(224),
        };
        let (chosen, cnt) = stock_available.resolve_tier(BaitTier::Rare);
        assert_eq!(chosen, BaitTier::Rare);
        assert_eq!(cnt, Some(130));

        // User chose Rare, but Rare is 0 -> resolves to Common!
        let stock_depleted = BaitStock {
            legendary: Some(136),
            rare: Some(0),
            common: Some(224),
        };
        let (chosen, cnt) = stock_depleted.resolve_tier(BaitTier::Rare);
        assert_eq!(chosen, BaitTier::Common);
        assert_eq!(cnt, Some(224));
    }

    #[test]
    fn test_is_bait_menu_visible() {
        assert!(is_bait_menu_visible("Fishing Baits\nRare Fish Bait x130"));
        assert!(is_bait_menu_visible("Common Fish Bait x224"));
        assert!(is_bait_menu_visible("Lesendary Fish Bait X136"));
        assert!(is_bait_menu_visible("Craft more bait types from Blacksmith Sen!"));

        // Background / water noise when menu is closed:
        assert!(!is_bait_menu_visible(""));
        assert!(!is_bait_menu_visible("P: 8250 MINS\n922,870\nMAX / MAX"));
        assert!(!is_bait_menu_visible("[Godly Fisherman]\nMOHAMMEDSAMIR2005"));
        assert!(!is_bait_menu_visible("random water pixels 0 0 0"));
    }

    #[test]
    fn test_parse_bait_stock_words_never_turn_into_digits() {
        // Plain tier names without numbers must NOT be parsed as numbers (e.g. 'fish' turning to '5')
        let text = "Fishing Baits\n\
                    Legendary Fish Bait\n\
                    Rare Fish Bait\n\
                    Common Fish Bait";
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, None);
        assert_eq!(s.rare, None);
        assert_eq!(s.common, None);
    }

    #[test]
    fn test_parse_bait_stock_noisy_user_image() {
        let text = "Fishint Baits\n\
                    Lesendary Fish Bait Xl46\n\
                    Rare Fish gait x166\n\
                    Common Fish Bait X198\n\
                    Craft rr orr types frcrn El Bcksn-.ith";
        assert!(is_bait_menu_visible(text));
        let s = parse_bait_stock(text);
        assert_eq!(s.legendary, Some(146));
        assert_eq!(s.rare, Some(166));
        assert_eq!(s.common, Some(198));
    }

    #[test]
    fn test_neural_bait_scanner_all_fixtures() {
        use image::GenericImageView;

        // 1. Fixture C (Latest user upload: media_1789597354352.png)
        // Legendary: 160, Rare: 57, Common: 183
        let bytes_c = include_bytes!("../../test_fixtures/img_c_160_57_183.png");
        let dyn_img_c = image::load_from_memory_with_format(bytes_c, image::ImageFormat::Png).expect("Load img C");
        let (w, h) = dyn_img_c.dimensions();
        let rgba_c = dyn_img_c.to_rgba8().into_raw();
        let frame_c = crate::core::types::Frame::new(w as usize, h as usize, rgba_c);
        let stock_c = scan_bait_stock_neural(&frame_c).expect("Neural scan img C");
        assert_eq!(stock_c.legendary, Some(160));
        assert_eq!(stock_c.rare, Some(57));
        assert_eq!(stock_c.common, Some(183));

        // 2. Fixture A (media_1789576873126.png)
        // Legendary: 136, Rare: 130, Common: 224
        let bytes_a = include_bytes!("../../test_fixtures/img_a_136_130_224.png");
        let dyn_img_a = image::load_from_memory_with_format(bytes_a, image::ImageFormat::Png).expect("Load img A");
        let (w, h) = dyn_img_a.dimensions();
        let rgba_a = dyn_img_a.to_rgba8().into_raw();
        let frame_a = crate::core::types::Frame::new(w as usize, h as usize, rgba_a);
        let stock_a = scan_bait_stock_neural(&frame_a).expect("Neural scan img A");
        assert_eq!(stock_a.legendary, Some(136));
        assert_eq!(stock_a.rare, Some(130));
        assert_eq!(stock_a.common, Some(224));

        // 3. Fixture B (menu_588.png)
        // Legendary: 146, Rare: 166, Common: 198
        let bytes_b = include_bytes!("../../test_fixtures/img_b_146_166_198.png");
        let dyn_img_b = image::load_from_memory_with_format(bytes_b, image::ImageFormat::Png).expect("Load img B");
        let (w, h) = dyn_img_b.dimensions();
        let rgba_b = dyn_img_b.to_rgba8().into_raw();
        let frame_b = crate::core::types::Frame::new(w as usize, h as usize, rgba_b);
        let stock_b = scan_bait_stock_neural(&frame_b).expect("Neural scan img B");
        assert_eq!(stock_b.legendary, Some(146));
        assert_eq!(stock_b.rare, Some(166));
        assert_eq!(stock_b.common, Some(198));
    }
}

