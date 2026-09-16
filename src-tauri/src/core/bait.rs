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
    // Look for 'x' or '×' followed by digits
    if let Some(idx) = lower.find(|c| c == 'x' || c == '×') {
        let after = &lower[idx + 1..];
        let digits: String = after
            .chars()
            .skip_while(|c| c.is_whitespace() || *c == ':')
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            return Some(n);
        }
    }

    // Fallback: look for trailing digits at the end of the line
    let trimmed = lower.trim_end();
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if !digits.is_empty() {
        if let Ok(n) = digits.parse::<u32>() {
            return Some(n);
        }
    }

    None
}

/// Parses in-game OCR text from the "Fishing Baits" menu.
/// Handles cases like:
///   "Legendary Fish Bait x104"
///   "Rare Fish Bait x35"
///   "Common Fish Bait x281"
pub fn parse_bait_stock(text: &str) -> BaitStock {
    let mut stock = BaitStock::default();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        let count = extract_quantity(trimmed);

        if lower.contains("legendary") {
            if count.is_some() || stock.legendary.is_none() {
                stock.legendary = count;
            }
        } else if lower.contains("rare") {
            if count.is_some() || stock.rare.is_none() {
                stock.rare = count;
            }
        } else if lower.contains("common") {
            if count.is_some() || stock.common.is_none() {
                stock.common = count;
            }
        }
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
}
