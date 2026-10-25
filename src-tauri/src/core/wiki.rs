//! Opt-in GPO Wiki import (MediaWiki API → [`ImportEntity`] → knowledge DB).
//!
//! Source: Grand Piece Online Wiki (Fandom), a public MediaWiki with a
//! structured `api.php` (generator MediaWiki 1.43, verified 2026-10-02).
//! No scraping of rendered HTML: only the stable API (`categorymembers`,
//! `revisions` wikitext) is used, sequentially with delays, a descriptive
//! user agent, and per-request timeouts. Live network NEVER runs in tests
//! (fixtures only); the app only fetches on explicit user sync.
//!
//! Mapping (validated against in-game data):
//! `{{Rarity|C|R|E|L|M}}` → Common/Rare/Epic/Legendary/Mythical,
//! `"Bari Bari no Mi"` → canonical `Bari`, image `File:` name retained as a
//! visual reference, first paragraph cleaned into `description`.
//! Every entity keeps `provenance.source = "wiki:grand-piece-online…"`.

use serde_json::Value;

use crate::core::knowledge::{EntityCategory, EntityProvenance, ImportEntity};

pub const WIKI_API: &str = "https://grand-piece-online.fandom.com/api.php";
pub const WIKI_SOURCE: &str = "wiki:grand-piece-online.fandom.com";
pub const WIKI_UA: &str = "GPO-Autofish-KB/1.0 (opt-in wiki sync)";
pub const DEVIL_FRUIT_CATEGORY: &str = "Devil Fruits";

/// Max fruit pages fetched per sync (respectful sequential fetching).
pub const MAX_PAGES_PER_SYNC: usize = 60;

#[derive(Debug, Clone, Default)]
pub struct WikiSyncReport {
    pub category_titles: usize,
    pub fetched_pages: usize,
    pub truncated: bool,
    pub parsed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(WIKI_UA)
        .build()
        .map_err(|e| format!("wiki http client: {e}"))
}

/// Titles (ns 0) of a category. Network errors are returned, never panics.
pub fn fetch_category_titles(category: &str, limit: u32) -> Result<Vec<String>, String> {
    let http = client()?;
    let resp = http
        .get(WIKI_API)
        .query(&[
            ("action", "query"),
            ("list", "categorymembers"),
            ("cmtitle", &format!("Category:{category}")),
            ("cmnamespace", "0"),
            ("cmlimit", &limit.min(200).to_string()),
            ("format", "json"),
        ])
        .send()
        .map_err(|e| format!("wiki category request failed: {e}"))?;
    parse_category_titles(&resp.text().map_err(|e| format!("wiki category body: {e}"))?)
}

/// Raw wikitext (section 0) of one page. `Ok(None)` = missing page.
pub fn fetch_page_wikitext(title: &str) -> Result<Option<String>, String> {
    let http = client()?;
    let resp = http
        .get(WIKI_API)
        .query(&[
            ("action", "query"),
            ("prop", "revisions"),
            ("rvprop", "content"),
            ("rvslots", "main"),
            ("rvsection", "0"),
            ("titles", title),
            ("format", "json"),
            ("formatversion", "2"),
        ])
        .send()
        .map_err(|e| format!("wiki page request failed: {e}"))?;
    parse_page_wikitext(&resp.text().map_err(|e| format!("wiki page body: {e}"))?)
}

pub fn parse_category_titles(body: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("wiki category JSON: {e}"))?;
    let members = v
        .pointer("/query/categorymembers")
        .and_then(|m| m.as_array())
        .ok_or_else(|| "wiki category response has no query.categorymembers".to_string())?;
    Ok(members
        .iter()
        .filter_map(|m| {
            (m.get("ns")?.as_u64()? == 0)
                .then(|| m.get("title")?.as_str().map(|s| s.to_string()))
                .flatten()
        })
        .collect())
}

pub fn parse_page_wikitext(body: &str) -> Result<Option<String>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("wiki page JSON: {e}"))?;
    let pages = v
        .pointer("/query/pages")
        .and_then(|p| p.as_array())
        .ok_or_else(|| "wiki page response has no query.pages".to_string())?;
    let page = pages.first().ok_or_else(|| "wiki page response has no pages".to_string())?;
    if page.get("missing").is_some() {
        return Ok(None);
    }
    Ok(page
        .pointer("/revisions/0/slots/main/content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string()))
}

pub fn rarity_code_to_name(code: &str) -> Option<&'static str> {
    match code.trim().to_ascii_uppercase().as_str() {
        "C" => Some("Common"),
        "R" => Some("Rare"),
        "E" => Some("Epic"),
        "L" => Some("Legendary"),
        "M" => Some("Mythical"),
        _ => None,
    }
}

fn extract_rarity(wikitext: &str) -> Option<String> {
    // `{{Rarity|X}}` or `|rarity = {{Rarity|X}}` / `|rarity=X`.
    for marker in ["{{Rarity|", "{{rarity|"] {
        let mut search = wikitext;
        while let Some(i) = search.find(marker) {
            let rest = &search[i + marker.len()..];
            let code: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            if let Some(name) = rarity_code_to_name(&code) {
                return Some(name.to_string());
            }
            search = rest;
        }
    }
    None
}

fn extract_image(wikitext: &str) -> Option<String> {
    // `|image=Foo.png` (first occurrence, gallery blocks skipped).
    let mut search = wikitext;
    while let Some(i) = search.find("|image=") {
        let rest = search[i + 7..].trim_start().to_string();
        if rest.starts_with('<') {
            search = &search[i + 7..];
            continue;
        }
        let name: String = rest
            .chars()
            .take_while(|c| !matches!(c, '|' | '\n' | '\r' | '}' | '<'))
            .collect::<String>()
            .trim()
            .to_string();
        if !name.is_empty() && (name.contains('.') || name.contains(' ')) {
            return Some(name);
        }
        search = &search[i + 7..];
    }
    None
}

/// First paragraph with wiki markup stripped. Returns None when nothing
/// readable remains (caller reports it as skipped, not merged).
pub fn clean_description(wikitext: &str) -> Option<String> {
    // Text before the first heading.
    let head = wikitext.split("==").next().unwrap_or("").to_string();
    // Drop the infobox template block (balanced-ish: cut at first "\nThe " or
    // first "\n\n" after templates... simpler: remove {{...}} spans).
    let mut s = String::new();
    let mut depth = 0usize;
    let chars: Vec<char> = head.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1) == Some(&'{') {
            depth += 1;
            i += 2;
        } else if chars[i] == '}' && chars.get(i + 1) == Some(&'}') {
            depth = depth.saturating_sub(1);
            i += 2;
        } else {
            if depth == 0 {
                s.push(chars[i]);
            }
            i += 1;
        }
    }
    // Strip refs, links, bold/italic, extra whitespace.
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut j = 0;
    while j < s.len() {
        if s[j..].starts_with("<ref") {
            if let Some(end) = s[j..].find("</ref>") {
                j += end + 6;
                continue;
            } else if let Some(end) = s[j..].find("/>") {
                j += end + 2;
                continue;
            }
        }
        if s[j..].starts_with("[[") {
            if let Some(end) = s[j..].find("]]") {
                let inner = &s[j + 2..j + end];
                let shown = inner.rsplit('|').next().unwrap_or(inner);
                out.push_str(shown);
                j += end + 2;
                continue;
            }
        }
        if s[j..].starts_with("'''") || s[j..].starts_with("''") {
            j += if s[j..].starts_with("'''") { 3 } else { 2 };
            continue;
        }
        let ch = s[j..].chars().next().unwrap();
        out.push(ch);
        j += ch.len_utf8();
        let _ = bytes;
    }
    let cleaned = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() < 20 {
        return None;
    }
    let mut clipped: String = cleaned.chars().take(500).collect();
    if clipped.len() < cleaned.len() {
        clipped.push('…');
    }
    Some(clipped)
}

/// Short in-game style name from a wiki title: "Bari Bari no Mi" → "Bari".
pub fn short_name_from_title(title: &str) -> Option<String> {
    let t = title.trim().strip_suffix(" no Mi").unwrap_or(title.trim());
    let first = t.split_whitespace().next()?.trim_matches(|c: char| !c.is_alphanumeric());
    if first.len() < 2 {
        return None;
    }
    Some(first.to_string())
}

/// Normalize one fruit page into an import record. `None` = unparseable
/// (reported as skipped with the title, never merged). The infobox image
/// filename is retained as a visual reference (page URL in provenance).
pub fn parse_fruit_page(title: &str, wikitext: &str) -> Option<ImportEntity> {
    let canonical = short_name_from_title(title)?;
    let page_url = format!(
        "https://grand-piece-online.fandom.com/wiki/{}",
        title.replace(' ', "_")
    );
    Some(ImportEntity {
        id: format!("fruit:{}", canonical.to_ascii_lowercase()),
        canonical_name: canonical,
        aliases: Vec::new(),
        category: EntityCategory::Fruit.as_str().to_string(),
        rarity: extract_rarity(wikitext),
        description: clean_description(wikitext),
        ocr_aliases: Vec::new(),
        visual_refs: extract_image(wikitext).into_iter().collect(),
        provenance: EntityProvenance {
            source: WIKI_SOURCE.to_string(),
            url: Some(page_url),
            updated: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARI: &str = "{{Demon Fruit|image=D792-C975-4047-4107-A1-AE-05-D7-FC78012-C.png|caption=\"x\"|type=Paramecia|rarity={{Rarity|R}}}}\nThe '''Bari Bari no Mi''' (''Barrier-Barrier Fruit'') is a {{Rarity|R}} Paramecia-type [[Devil Fruits|Devil Fruit]] that gives its user barriers.\n\n== Moveset ==";
    const PIKA: &str = "{{Demon Fruit\n| title = Pika Pika no Mi\n| image = pikaHD.png\n| type = Logia\n| rarity = {{Rarity|L}}\n}}\nThe '''Pika Pika no Mi''' (''Glint-Glint Fruit'') is light.\n\nMore text here to pass the length gate for descriptions.";

    #[test]
    fn rarity_codes_match_ingame_data() {
        assert_eq!(rarity_code_to_name("C"), Some("Common"));
        assert_eq!(rarity_code_to_name("R"), Some("Rare"));
        assert_eq!(rarity_code_to_name("E"), Some("Epic"));
        assert_eq!(rarity_code_to_name("L"), Some("Legendary"));
        assert_eq!(rarity_code_to_name("M"), Some("Mythical"));
        assert_eq!(rarity_code_to_name("X"), None);
        assert_eq!(extract_rarity(BARI).as_deref(), Some("Rare"));
        assert_eq!(extract_rarity(PIKA).as_deref(), Some("Legendary"));
    }

    #[test]
    fn fruit_page_parses_with_provenance() {
        let e = parse_fruit_page("Bari Bari no Mi", BARI).expect("parse");
        assert_eq!(e.canonical_name, "Bari");
        assert_eq!(e.id, "fruit:bari");
        assert_eq!(e.rarity.as_deref(), Some("Rare"));
        assert_eq!(e.category, "fruit");
        assert_eq!(e.provenance.source, WIKI_SOURCE);
        assert!(e.provenance.url.unwrap().contains("Bari_Bari_no_Mi"));
        let d = e.description.expect("description");
        assert!(d.contains("Barrier-Barrier Fruit"));
        assert!(!d.contains("{{") && !d.contains("[["));
    }

    #[test]
    fn short_names_and_garbage() {
        assert_eq!(short_name_from_title("Bomu Bomu no Mi").as_deref(), Some("Bomu"));
        assert_eq!(short_name_from_title("Yomi Yomi no Mi").as_deref(), Some("Yomi"));
        assert!(short_name_from_title("X").is_none());
        assert!(short_name_from_title("").is_none());
        assert!(parse_fruit_page("X", "no infobox here {{ nope").is_none()
            || parse_fruit_page("X", "no infobox here").is_none());
    }

    #[test]
    fn category_and_missing_pages() {
        let body = r#"{"query":{"categorymembers":[{"pageid":1,"ns":0,"title":"Bari Bari no Mi"},{"pageid":2,"ns":10,"title":"Template:X"},{"pageid":3,"ns":0,"title":"Devil Fruits"}]}}"#;
        let titles = parse_category_titles(body).unwrap();
        assert_eq!(titles, vec!["Bari Bari no Mi".to_string(), "Devil Fruits".to_string()]);
        assert!(parse_category_titles(r#"{"query":{}}"#).is_err());
        let missing = r#"{"query":{"pages":[{"ns":0,"title":"Nope","missing":true}]}}"#;
        assert_eq!(parse_page_wikitext(missing).unwrap(), None);
        assert!(parse_page_wikitext("garbage").is_err());
    }

    /// Live validation against the real GPO Wiki API. NEVER runs in CI
    /// (`#[ignore]`): requires network and hits a third-party server.
    /// Run explicitly: `cargo test --lib wiki::tests::live_wiki_smoke -- --ignored`.
    #[test]
    #[ignore]
    fn live_wiki_smoke() {
        let titles = fetch_category_titles("Devil Fruits", 5).expect("category fetch");
        assert!(!titles.is_empty(), "category must list fruit pages");
        let mut parsed = 0;
        for title in titles.iter().take(3) {
            if let Some(wikitext) = fetch_page_wikitext(title).expect("page fetch") {
                if parse_fruit_page(title, &wikitext).is_some() {
                    parsed += 1;
                }
            }
        }
        assert!(parsed > 0, "at least one real page must parse");
    }

    #[test]
    fn image_extraction_skips_galleries() {
        assert_eq!(extract_image(BARI).as_deref(), Some("D792-C975-4047-4107-A1-AE-05-D7-FC78012-C.png"));
        let gal = "|image=<gallery widths=\"185\">\nFoo.png|Game\n</gallery>";
        assert_eq!(extract_image(gal), None);
    }
}
