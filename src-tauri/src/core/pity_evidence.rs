//! Strict, local OCR evidence for a confirmed fruit catch.
//! Never infer pity from the session counter or an unrelated fraction in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PityEvidence {
    pub current: u32,
    pub total: Option<u32>,
}
impl PityEvidence {
    pub fn text(self) -> String {
        match self.total {
            Some(total) => format!("{}/{total}", self.current),
            None => self.current.to_string(),
        }
    }
}

fn counter_number(token: &str) -> Option<u32> {
    if token.is_empty() || token.len() > 3 {
        return None;
    }
    let clean: String = token
        .chars()
        .map(|c| if c == 'o' { '0' } else { c })
        .collect();
    if !clean.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    clean.parse().ok()
}

/// Existing structured nonzero evidence is conservative: never replace it
/// with a different zero counter found elsewhere in a full-screen scan.
pub fn recorded_nonzero(raw: Option<&str>) -> Option<PityEvidence> {
    let raw = raw?.trim();
    let (current, total) = raw.split_once('/')?;
    let current = counter_number(current.trim())?;
    let total = counter_number(total.trim())?;
    if current == 0 || !matches!(total, 40 | 100) || current > total {
        return None;
    }
    Some(PityEvidence {
        current,
        total: Some(total),
    })
}

pub fn parse(raw: &str) -> Option<PityEvidence> {
    let mut normalized = String::with_capacity(raw.len());
    for character in raw.chars().flat_map(char::to_lowercase) {
        if character == '/' {
            normalized.push_str(" / ");
        } else if character.is_ascii_alphanumeric() {
            normalized.push(character);
        } else {
            normalized.push(' ');
        }
    }
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let mut evidence: Option<PityEvidence> = None;
    let mut selected_priority = 0u8;
    for (index, word) in words.iter().enumerate() {
        if !matches!(*word, "pity" | "legendarypity" | "guaranteedpity") {
            continue;
        }
        if index > 0 && matches!(words[index - 1], "fruit" | "fruits" | "common" | "rare") {
            continue;
        }
        let legendary = matches!(*word, "legendarypity" | "guaranteedpity")
            || (index > 0 && matches!(words[index - 1], "legendary" | "guaranteed"));
        let priority = if legendary { 2 } else { 1 };
        let Some(current) = words.get(index + 1).and_then(|word| counter_number(word)) else {
            continue;
        };
        let total = if words.get(index + 2) == Some(&"/") {
            let Some(total) = words.get(index + 3).and_then(|word| counter_number(word)) else {
                continue;
            };
            if !matches!(total, 40 | 100) || current > total {
                continue;
            }
            Some(total)
        } else {
            // A lone zero next to an unqualified Pity label could be a different
            // game counter; require the explicit Legendary qualifier here.
            if !legendary || current > 100 {
                continue;
            }
            None
        };
        let candidate = PityEvidence { current, total };
        if priority < selected_priority {
            continue;
        }
        if priority > selected_priority {
            evidence = None;
            selected_priority = priority;
        }
        match evidence {
            None => evidence = Some(candidate),
            Some(previous) => {
                if previous.current != current
                    || (previous.total.is_some() && total.is_some() && previous.total != total)
                {
                    return None;
                }
                if previous.total.is_none() {
                    evidence = Some(candidate);
                }
            }
        }
    }
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_only_labelled_zero_pity() {
        for text in [
            "Legendary Pity 0/40",
            "Pity: 0 / 100",
            "LegendaryPity O / 4O",
            "Legendary Pity\n0/40",
        ] {
            assert_eq!(parse(text).unwrap().current, 0, "{text}");
        }
        assert_eq!(parse("Legendary Pity 0").unwrap().text(), "0");
    }
    #[test]
    fn unreadable_or_absent_pity_remains_unknown() {
        for text in [
            "YOU GOT A DEVIL FRUIT DROP CHECK YOUR BA",
            "Legendary Pity unreadable",
            "Pity 0",
            "Pity 0 /",
            "Pity 0/4",
            "Pity 0/300",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
    #[test]
    fn nonzero_evidence_is_preserved() {
        assert_eq!(
            parse("Legendary Pity 17 / 40"),
            Some(PityEvidence {
                current: 17,
                total: Some(40)
            })
        );
        assert_eq!(parse("Legendary Pity 10").unwrap().text(), "10");
    }
    #[test]
    fn unrelated_zeros_and_ratios_do_not_become_pity() {
        for text in [
            "HP 0/100 Level 0/40 Backpack 0",
            "Pity unreadable Health 0/100",
            "Pity timer 0/40",
            "Pity 0/5",
            "Fruit Pity 0/10",
            "Legendary 0/40 Pity unknown",
            "Pity 12/40 Level 0/100",
        ] {
            let found = parse(text);
            assert!(!found.is_some_and(|value| value.current == 0), "{text}");
        }
    }
    #[test]
    fn legendary_counter_has_priority_over_regular_fruit_counter() {
        assert_eq!(
            parse("Fruit Pity 0/40 Legendary Pity 17/40")
                .unwrap()
                .current,
            17
        );
        assert_eq!(parse("Pity 0/40 Legendary Pity 17/40").unwrap().current, 17);
        assert_eq!(parse("Legendary Pity 17/40 Pity 0/40").unwrap().current, 17);
        assert_eq!(parse("Guaranteed Pity 0/100").unwrap().current, 0);
        assert_eq!(recorded_nonzero(Some("17/40")).unwrap().current, 17);
        assert_eq!(recorded_nonzero(Some("0/40")), None);
    }
    #[test]
    fn conflicting_counters_are_not_guessed() {
        assert_eq!(parse("Pity 0/40 Chat Pity 12/40"), None);
        assert_eq!(parse("Legendary Pity 0/40 Legendary Pity 0/100"), None);
        assert_eq!(
            parse("Legendary Pity 0 Legendary Pity 0/40")
                .unwrap()
                .text(),
            "0/40"
        );
    }
}
