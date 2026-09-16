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


fn extract_quantity(line: &str) -> Option<u32> {
    let lower = line.to_lowercase();

    // 1. Scan tokens from right to left for standard "x123", "*123", ":123", "123"
    for word in lower.split_whitespace().rev() {
        let clean: String = word.chars().filter(|c| c.is_ascii_digit()).collect();
        if !clean.is_empty() {
            if let Ok(n) = clean.parse::<u32>() {
                if n <= 9999 {
                    return Some(n);
                }
            }
        }
    }

    // 2. Look for explicit multiplier symbols: 'x', '×', '*', '•', '+', ':' followed by digits
    for sym in ['x', '×', '*', '•', '+', ':'] {
        if let Some(idx) = lower.rfind(sym) {
            let after = &lower[idx + 1..];
            let digits: String = after
                .chars()
                .skip_while(|c| c.is_whitespace() || *c == ':' || *c == '.' || *c == '-' || *c == '\'')
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u32>() {
                if n <= 9999 {
                    return Some(n);
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
        if lower.contains("fishing") && lower.contains("bait") && !lower.contains("x") && !lower.contains("*") {
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
            || (lower.starts_with('l') && (lower.contains("sh") || lower.contains("bait") || lower.contains("fish")));

        let is_rare = lower.contains("rare")
            || lower.contains("rar")
            || lower.contains("r.are")
            || (lower.starts_with('r') && (lower.contains("bait") || lower.contains("fish")));

        let is_common = lower.contains("common")
            || lower.contains("comon")
            || lower.contains("comm")
            || lower.contains("mmon")
            || lower.contains("ommon")
            || (lower.starts_with('c') && (lower.contains("bait") || lower.contains("fish")));

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
}
