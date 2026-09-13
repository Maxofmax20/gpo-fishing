use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;

pub const DEFAULT_FRUITS: &[&str] = &[
    "Tori", "Mochi", "Ope", "Venom", "Buddha", "Pteranodon", "Smoke", "Goru", "Yuki", "Yami",
    "Pika", "Magu", "Kage", "Mera", "Paw", "Goro", "Ito", "Hie", "Suna", "Gura", "Zushi", "Kira",
    "Spring", "Yomi", "Bomb", "Gomu", "Horo", "Mero", "Bari", "Heal", "Spin", "Suke", "Kilo",
    "Dragon", "Uo", "Phoenix", "Light", "Dark", "Magma", "Flame", "Ice", "Shadow", "String",
    "Sand", "Gravity", "Tremor", "Rumble", "Clear", "Barrier", "Love", "Rubber", "Bomu",
    "Revive", "Chiyu", "Guru", "Glint", "Soru", "Soul", "Leopard", "Neko", "Lucci", "Bisu",
    "Biscuit", "Moku", "Plume", "Bane", "Doku", "Daibutsu", "Hito", "Seiryu", "Nikyu", "Kaido",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FruitRarity {
    Common,
    Rare,
    Epic,
    Legendary,
    Mythical,
    Unknown,
}

impl FruitRarity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Rare => "Rare",
            Self::Epic => "Epic",
            Self::Legendary => "Legendary",
            Self::Mythical => "Mythical",
            Self::Unknown => "Unknown",
        }
    }

    pub fn is_high_tier(&self) -> bool {
        matches!(self, Self::Legendary | Self::Mythical)
    }
}

pub fn fruit_rarity(name: &str) -> FruitRarity {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        // Mythical (9)
        "tori" | "phoenix" | "mochi" | "ope" | "venom" | "doku" | "buddha" | "daibutsu" | "hito"
        | "pteranodon" | "ryu" | "ptera" | "dragon" | "seiryu" | "uo" | "kaido" | "soru" | "soul"
        | "leopard" | "neko" | "lucci" => FruitRarity::Mythical,

        // Legendary (16)
        "pika" | "glint" | "magu" | "magma" | "hie" | "ice" | "goro" | "rumble" | "lightning"
        | "mera" | "flame" | "fire" | "suna" | "sand" | "yami" | "dark" | "darkness" | "yuki"
        | "snow" | "moku" | "smoke" | "plume" | "gura" | "tremor" | "quake" | "zushi" | "gravity"
        | "paw" | "nikyu" | "ito" | "string" | "kage" | "shadow" | "goru" | "gold" | "bisu"
        | "biscuit" => FruitRarity::Legendary,

        // Epic (3)
        "yomi" | "revive" | "skeleton" | "spring" | "bane" | "kira" | "diamond" => FruitRarity::Epic,

        // Rare (5)
        "gomu" | "rubber" | "bomb" | "bomu" | "bari" | "barrier" | "mero" | "love" | "horo"
        | "hollow" | "ghost" => FruitRarity::Rare,

        // Common (4)
        "kilo" | "weight" | "suke" | "clear" | "invisible" | "spin" | "guru" | "heal" | "chiyu" => {
            FruitRarity::Common
        }

        _ => {
            if lower.contains("phoenix") || lower.contains("tori") || lower.contains("mochi")
                || lower.contains("ope") || lower.contains("venom") || lower.contains("doku")
                || lower.contains("buddha") || lower.contains("daibutsu") || lower.contains("pteranodon")
                || lower.contains("dragon") || lower.contains("seiryu") || lower.contains("soru")
                || lower.contains("leopard") {
                FruitRarity::Mythical
            } else if lower.contains("pika") || lower.contains("magu") || lower.contains("hie")
                || lower.contains("goro") || lower.contains("mera") || lower.contains("suna")
                || lower.contains("yami") || lower.contains("yuki") || lower.contains("smoke")
                || lower.contains("moku") || lower.contains("gura") || lower.contains("zushi")
                || lower.contains("nikyu") || lower.contains("paw") || lower.contains("ito")
                || lower.contains("kage") || lower.contains("goru") || lower.contains("bisu") {
                FruitRarity::Legendary
            } else if lower.contains("yomi") || lower.contains("bane") || lower.contains("kira") {
                FruitRarity::Epic
            } else if lower.contains("gomu") || lower.contains("bomu") || lower.contains("bomb")
                || lower.contains("bari") || lower.contains("mero") || lower.contains("horo") {
                FruitRarity::Rare
            } else if lower.contains("kilo") || lower.contains("suke") || lower.contains("spin")
                || lower.contains("chiyu") {
                FruitRarity::Common
            } else {
                FruitRarity::Unknown
            }
        }
    }
}

pub fn is_legendary_or_mythical(name: &str) -> bool {
    fruit_rarity(name).is_high_tier()
}

pub const DEFAULT_DROP_PHRASES: &[&str] = &[
    "devil fruit",
    "fished up a devil",
    "got a devil fruit",
    "devil fruit drop",
    "check your backpack",
];

pub const DEFAULT_DROP_KEYWORDS: &[&str] = &["devil", "fruit", "backpack", "drop", "got", "fished up"];

pub const DEFAULT_CATCH_PHRASES: &[&str] = &["you caught", "caught a", "you fished", "fished up", "you got", "you found", "new item"];

pub const DEFAULT_FAIL_PHRASES: &[&str] = &["got away", "escaped", "line broke", "line snapped", "lost the", "too slow"];

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "has", "have", "spawn", "spawned", "at", "in", "on", "new", "item", "fruit", "devil", "you", "your",
    "got", "town", "island", "studs", "none", "world", "fished", "up", "caught", "check", "backpack",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Lexicon {
    pub fruits: Vec<String>,
    pub drop_phrases: Vec<String>,
    pub drop_keywords: Vec<String>,
    pub spawn_keywords: Vec<String>,
    pub catch_phrases: Vec<String>,
    pub fail_phrases: Vec<String>,
    pub fuzzy_threshold: f64,
}

impl Default for Lexicon {
    fn default() -> Self {
        Self {
            fruits: DEFAULT_FRUITS.iter().map(|s| s.to_string()).collect(),
            drop_phrases: DEFAULT_DROP_PHRASES.iter().map(|s| s.to_string()).collect(),
            drop_keywords: DEFAULT_DROP_KEYWORDS.iter().map(|s| s.to_string()).collect(),
            spawn_keywords: vec!["spawned".into(), "spawn".into()],
            catch_phrases: DEFAULT_CATCH_PHRASES.iter().map(|s| s.to_string()).collect(),
            fail_phrases: DEFAULT_FAIL_PHRASES.iter().map(|s| s.to_string()).collect(),
            fuzzy_threshold: 0.85,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropInfo {
    pub text: String,
    pub is_legendary: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnInfo {
    pub text: String,
    pub name: Option<String>,
    pub location: Option<String>,
}

impl SpawnInfo {
    pub fn label(&self) -> String {
        match (&self.name, &self.location) {
            (Some(n), Some(l)) => format!("{n} at {l}"),
            (Some(n), None) => n.clone(),
            (None, Some(l)) => format!("Unknown fruit at {l}"),
            (None, None) => "Unknown fruit".into(),
        }
    }
}

pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        let c = if ch.is_alphanumeric() || ch == '/' { ch.to_ascii_lowercase() } else { ' ' };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn similar(a: &str, b: &str, thr: f64) -> bool {
    a == b || (a.len() >= 3 && jaro_winkler(a, b) >= thr)
}

fn word_at(words: &[&str], target: &str, thr: f64) -> Option<usize> {
    words.iter().position(|w| similar(w, target, thr))
}

fn match_fruit(lex: &Lexicon, word: &str) -> Option<String> {
    if word.len() < 3 {
        return None;
    }
    let lower = word.to_lowercase();
    if let Some(f) = lex.fruits.iter().find(|f| f.eq_ignore_ascii_case(word)) {
        return Some(f.clone());
    }
    if STOPWORDS.contains(&lower.as_str()) {
        return None;
    }
    let mut best: Option<(f64, &String)> = None;
    for f in &lex.fruits {
        let s = jaro_winkler(&lower, &f.to_lowercase());
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, f));
        }
    }
    match best {
        Some((s, f)) if s >= lex.fuzzy_threshold => Some(f.clone()),
        _ => None,
    }
}

fn first_fruit<'a>(lex: &Lexicon, words: impl Iterator<Item = &'a str>) -> Option<String> {
    words.filter_map(|w| match_fruit(lex, w)).next()
}

fn title_case(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn detect_drop(lex: &Lexicon, raw: &str) -> Option<DropInfo> {
    let text = normalize(raw);
    if text.is_empty() {
        return None;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let thr = lex.fuzzy_threshold.min(0.8);

    if mentions_spawn(lex, &words, thr) {
        return None;
    }
    if let Some(i) = word_at(&words, "item", thr) {
        let fruit_name = first_fruit(lex, words[i + 1..].iter().copied()).or_else(|| first_fruit(lex, words.iter().copied()))?;
        return Some(DropInfo { is_legendary: is_legendary(&words), text, name: Some(fruit_name) });
    }

    let by_phrase = lex.drop_phrases.iter().any(|p| text.contains(p.as_str()));
    let by_keywords = lex.drop_keywords.iter().filter(|k| text.contains(k.as_str())).count() >= 2;
    if !(by_phrase || by_keywords) {
        return None;
    }
    let fruit_name = first_fruit(lex, words.iter().copied());
    Some(DropInfo { is_legendary: is_legendary(&words), text, name: fruit_name })
}

fn mentions_spawn(lex: &Lexicon, words: &[&str], thr: f64) -> bool {
    lex.spawn_keywords.iter().any(|k| words.iter().any(|w| similar(w, k, thr)))
}

fn mentions_pity(words: &[&str]) -> bool {
    words.iter().any(|w| jaro_winkler(w, "pity") >= 0.85)
}

fn is_zero(w: &str) -> bool {
    matches!(w, "0" | "o" | "00")
}

fn is_legendary(words: &[&str]) -> bool {
    if let Some(i) = words.iter().position(|w| jaro_winkler(w, "pity") >= 0.85) {
        return words.get(i + 1).is_some_and(|w| is_zero(w));
    }
    words.iter().any(|w| {
        let mut it = w.splitn(2, '/');
        match (it.next(), it.next()) {
            (Some(z), Some(rest)) => is_zero(z) && rest.parse::<u32>().is_ok_and(|n| (1..=100).contains(&n)),
            _ => false,
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatchVerdict {
    Caught,
    Failed,
    Unknown,
}

pub fn detect_catch(lex: &Lexicon, raw: &str) -> CatchVerdict {
    let text = normalize(raw);
    if text.is_empty() {
        return CatchVerdict::Unknown;
    }
    if lex.fail_phrases.iter().any(|p| text.contains(p.as_str())) {
        return CatchVerdict::Failed;
    }
    if lex.catch_phrases.iter().any(|p| text.contains(p.as_str())) || detect_drop(lex, raw).is_some() {
        return CatchVerdict::Caught;
    }
    CatchVerdict::Unknown
}

/// Extracts the category ("fish" | "fruit") and specific item name from catch OCR text.
pub fn parse_catch_item(lex: &Lexicon, raw: &str) -> (String, String) {
    let clean = raw.trim();
    if clean.is_empty() {
        return ("fish".into(), "Fish".into());
    }

    // 1. Check if it is a devil fruit
    if let Some(drop) = detect_drop(lex, raw) {
        let name = drop.name.unwrap_or_else(|| {
            if drop.is_legendary {
                "Legendary Devil Fruit".into()
            } else {
                "Devil Fruit".into()
            }
        });
        return ("fruit".into(), name);
    }

    // Check if any known fruit name appears in raw text
    let norm = normalize(raw);
    let words: Vec<&str> = norm.split_whitespace().collect();
    if let Some(fruit_name) = first_fruit(lex, words.iter().copied()) {
        return ("fruit".into(), fruit_name);
    }

    // 2. Extract item between < > or ( ) or [ ]
    if let Some(start) = clean.find('<').or_else(|| clean.find('(')).or_else(|| clean.find('[')) {
        if let Some(end) = clean[start + 1..].find('>').or_else(|| clean[start + 1..].find(')')).or_else(|| clean[start + 1..].find(']')) {
            let inside = clean[start + 1..start + 1 + end].trim();
            if !inside.is_empty() {
                let in_norm = normalize(inside);
                let in_words: Vec<&str> = in_norm.split_whitespace().collect();
                if let Some(fn_name) = first_fruit(lex, in_words.iter().copied()) {
                    return ("fruit".into(), fn_name);
                }
                return ("fish".into(), title_case(&inside.split_whitespace().collect::<Vec<_>>()));
            }
        }
    }

    // 3. Extract after "caught a", "caught an", "fished up a", "got a", "found a", etc.
    let lower = clean.to_lowercase();
    for prefix in &[
        "caught a ", "caught an ", "caught ",
        "fished up a ", "fished up an ", "fished up ",
        "got a ", "got an ", "found a ", "found an "
    ] {
        if let Some(idx) = lower.find(prefix) {
            let after = &clean[idx + prefix.len()..];
            let end_idx = after.find(|c| c == '.' || c == '!' || c == '?' || c == '\n').unwrap_or(after.len());
            let name = after[..end_idx].trim();
            if !name.is_empty() {
                let w: Vec<&str> = name.split_whitespace().collect();
                return ("fish".into(), title_case(&w));
            }
        }
    }

    ("fish".into(), "Fish".into())
}

pub fn detect_spawn(lex: &Lexicon, raw: &str) -> Option<SpawnInfo> {
    let text = normalize(raw);
    if text.is_empty() {
        return None;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let thr = lex.fuzzy_threshold.min(0.8);
    if mentions_pity(&words) {
        return None;
    }

    let has_keyword = lex.spawn_keywords.iter().any(|k| words.iter().any(|w| similar(w, k, thr)) || text.replace(' ', "").contains(k.as_str()));
    let has_i = word_at(&words, "has", 0.9);
    let at_i = has_i.and_then(|h| words[h + 1..].iter().position(|w| *w == "at").map(|p| h + 1 + p));

    let name_before_has = has_i.and_then(|h| {
        let start = words[..h].iter().rposition(|w| *w == "a" || *w == "an").map(|p| p + 1).unwrap_or(h.saturating_sub(2));
        first_fruit(lex, words[start..h].iter().rev().copied())
    });
    let structural = name_before_has.is_some() && has_i.is_some();
    if !has_keyword && !structural {
        return None;
    }
    let name = name_before_has.or_else(|| first_fruit(lex, words.iter().copied()));
    let location = at_i.and_then(|a| {
        let loc: Vec<&str> = words[a + 1..]
            .iter()
            .take_while(|w| w.chars().all(|c| c.is_ascii_alphabetic()) && !matches!(**w, "studs" | "none" | "has" | "spawned"))
            .copied()
            .take(3)
            .collect();
        (!loc.is_empty()).then(|| title_case(&loc))
    });
    Some(SpawnInfo { text, name, location })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> Lexicon {
        Lexicon::default()
    }

    #[test]
    fn drop_by_phrase() {
        let d = detect_drop(&lex(), "You fished up a Devil Fruit! Check your backpack").unwrap();
        assert!(!d.is_legendary);
    }

    #[test]
    fn drop_new_item() {
        for raw in ["New Item <Buddha>", "New Item <Suna>", "New ltem (Buddha>", "NEW ITEM <Pteranodn>", "New Item < Sprng >"] {
            assert!(detect_drop(&lex(), raw).is_some(), "{raw}");
        }
    }

    #[test]
    fn drop_pity_decides_legendary() {
        let l = lex();
        let d = detect_drop(&l, "g eye you got a devil fruwdrop check your legendary pity 17").unwrap();
        assert!(!d.is_legendary);
        let d = detect_drop(&l, "you got a devil fruit drop check your backpack legendary pity 0").unwrap();
        assert!(d.is_legendary);
        let d = detect_drop(&l, "you got a devil fruit drop legendary pity o").unwrap();
        assert!(d.is_legendary);
        let d = detect_drop(&l, "you got a devil fruit drop legendary pity 10").unwrap();
        assert!(!d.is_legendary);
        let d = detect_drop(&l, "You got a Devil Fruit drop! Legendary Pity 3").unwrap();
        assert!(!d.is_legendary);
    }

    #[test]
    fn drop_and_spawn_never_cross() {
        let l = lex();
        assert!(detect_drop(&l, "A Devil Fruit has spawned at MARINE FORD").is_none());
        assert!(detect_drop(&l, "A VENOM has spawned at ORANGE TOWN").is_none());
        assert!(detect_spawn(&l, "you got a devil fruit drop check your backpack legendary pity 0").is_none());
        assert!(detect_spawn(&l, "You got a Devil Fruit drop! Legendary Pity 17").is_none());
    }

    #[test]
    fn drop_new_item_fish_is_not_a_drop() {
        assert!(detect_drop(&lex(), "New Item <Tuna>").is_none());
        assert!(detect_drop(&lex(), "New Item <Shark>").is_none());
        assert!(detect_drop(&lex(), "New Item <Old Boot>").is_none());
    }

    #[test]
    fn drop_legendary_pity() {
        let d = detect_drop(&lex(), "DEVIL FRUIT drop 0/37").unwrap();
        assert!(d.is_legendary);
        let d = detect_drop(&lex(), "devil fruit 3/37").unwrap();
        assert!(!d.is_legendary);
    }

    #[test]
    fn drop_none_on_fish() {
        assert!(detect_drop(&lex(), "You caught a Tuna").is_none());
    }

    #[test]
    fn catch_verdicts() {
        let l = lex();
        assert_eq!(detect_catch(&l, "You caught a Tuna!"), CatchVerdict::Caught);
        assert_eq!(detect_catch(&l, "New Item <Tuna>"), CatchVerdict::Caught);
        assert_eq!(detect_catch(&l, "The fish got away..."), CatchVerdict::Failed);
        assert_eq!(detect_catch(&l, ""), CatchVerdict::Unknown);
        assert_eq!(detect_catch(&l, "random chat line"), CatchVerdict::Unknown);
    }

    #[test]
    fn spawn_named_with_location() {
        let s = detect_spawn(&lex(), "A VENOM has spawned at ORANGE TOWN").unwrap();
        assert_eq!(s.name.as_deref(), Some("Venom"));
        assert_eq!(s.location.as_deref(), Some("Orange Town"));
        assert_eq!(s.label(), "Venom at Orange Town");
    }

    #[test]
    fn spawn_named_with_overlapping_hud_text() {
        let s = detect_spawn(&lex(), "A VENOM has spaWned.at ORANGE TOWN 3473 Studs None").unwrap();
        assert_eq!(s.name.as_deref(), Some("Venom"));
        assert_eq!(s.location.as_deref(), Some("Orange Town"));
        let s = detect_spawn(&lex(), "A VENOM has sp@wned at ORANGE TOWN").unwrap();
        assert_eq!(s.name.as_deref(), Some("Venom"));
        let s = detect_spawn(&lex(), "3473 Studs A VENOM has spa None wned at ORANGE TOWN").unwrap();
        assert_eq!(s.name.as_deref(), Some("Venom"));
        assert_eq!(s.location.as_deref(), Some("Orange Town"));
    }

    #[test]
    fn spawn_fuzzy_name() {
        assert_eq!(detect_spawn(&lex(), "A Pteranodn has spavned").unwrap().name.as_deref(), Some("Pteranodon"));
        assert_eq!(detect_spawn(&lex(), "A Mera fruit has spawned!").unwrap().name.as_deref(), Some("Mera"));
        assert_eq!(detect_spawn(&lex(), "A MOCHl has spawned at SANDORA").unwrap().name.as_deref(), Some("Mochi"));
    }

    #[test]
    fn spawn_unnamed_falls_back() {
        let s = detect_spawn(&lex(), "A fruit has spawned").unwrap();
        assert_eq!(s.name, None);
        assert_eq!(s.label(), "Unknown fruit");
        let s = detect_spawn(&lex(), "A Devil Fruit has spawned at MARINE FORD").unwrap();
        assert_eq!(s.name, None);
        assert_eq!(s.label(), "Unknown fruit at Marine Ford");
        let s = detect_spawn(&lex(), "A ZXQW has spawned at ORANGE TOWN").unwrap();
        assert_eq!(s.name, None);
    }

    #[test]
    fn spawn_ignores_unrelated() {
        assert!(detect_spawn(&lex(), "Mera fruit is nice").is_none());
        assert!(detect_spawn(&lex(), "New Item <Buddha>").is_none());
        assert!(detect_spawn(&lex(), "You fished up a Devil Fruit").is_none());
        assert!(detect_spawn(&lex(), "").is_none());
    }

    #[test]
    fn parse_catch_items() {
        let l = lex();
        assert_eq!(parse_catch_item(&l, "New Item <Tuna>"), ("fish".into(), "Tuna".into()));
        assert_eq!(parse_catch_item(&l, "New Item <Buddha>"), ("fruit".into(), "Buddha".into()));
        assert_eq!(parse_catch_item(&l, "You caught a Tuna!"), ("fish".into(), "Tuna".into()));
        assert_eq!(parse_catch_item(&l, "You caught a Golden Fish!"), ("fish".into(), "Golden Fish".into()));
        assert_eq!(parse_catch_item(&l, "New Item <Colossal Shark>"), ("fish".into(), "Colossal Shark".into()));
        assert_eq!(parse_catch_item(&l, "You fished up a Devil Fruit! Check your backpack"), ("fruit".into(), "Devil Fruit".into()));
        assert_eq!(parse_catch_item(&l, "New Item <Mochi>"), ("fruit".into(), "Mochi".into()));
    }

    #[test]
    fn fruit_rarity_classification() {
        assert_eq!(fruit_rarity("Tori"), FruitRarity::Mythical);
        assert_eq!(fruit_rarity("Mochi"), FruitRarity::Mythical);
        assert_eq!(fruit_rarity("Ope"), FruitRarity::Mythical);
        assert_eq!(fruit_rarity("Pika"), FruitRarity::Legendary);
        assert_eq!(fruit_rarity("Magu"), FruitRarity::Legendary);
        assert_eq!(fruit_rarity("Goro"), FruitRarity::Legendary);
        assert_eq!(fruit_rarity("Yomi"), FruitRarity::Epic);
        assert_eq!(fruit_rarity("Gomu"), FruitRarity::Rare);
        assert_eq!(fruit_rarity("Kilo"), FruitRarity::Common);

        assert!(is_legendary_or_mythical("Tori"));
        assert!(is_legendary_or_mythical("Mochi"));
        assert!(is_legendary_or_mythical("Pika"));
        assert!(is_legendary_or_mythical("Zushi"));
        assert!(!is_legendary_or_mythical("Yomi"));
        assert!(!is_legendary_or_mythical("Gomu"));
        assert!(!is_legendary_or_mythical("Kilo"));
    }
}
