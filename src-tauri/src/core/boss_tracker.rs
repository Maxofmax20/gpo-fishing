use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BossId {
    HawkEye,
    Roger,
    SoulKing,
    RadiantAdmiral,
    TravellingMerchant,
}

impl BossId {
    pub fn all() -> &'static [BossId] {
        &[
            BossId::HawkEye,
            BossId::Roger,
            BossId::SoulKing,
            BossId::RadiantAdmiral,
            BossId::TravellingMerchant,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            BossId::HawkEye => "Hawk Eye (Mihawk)",
            BossId::Roger => "Roger",
            BossId::SoulKing => "Soul King (Brook)",
            BossId::RadiantAdmiral => "Radiant Admiral (Kizaru)",
            BossId::TravellingMerchant => "Travelling Merchant",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            BossId::HawkEye => "🦅",
            BossId::Roger => "👑",
            BossId::SoulKing => "🎺",
            BossId::RadiantAdmiral => "⚡",
            BossId::TravellingMerchant => "🛒",
        }
    }

    pub fn location(&self) -> &'static str {
        match self {
            BossId::HawkEye => "Umi Island (Second Sea)",
            BossId::Roger => "Umi Island (Second Sea)",
            BossId::SoulKing => "Soul King's Ship (Second Sea)",
            BossId::RadiantAdmiral => "Marine Base G-1 (First Sea)",
            BossId::TravellingMerchant => "Random Island (Check compass icon)",
        }
    }

    pub fn cycle_seconds(&self) -> i64 {
        match self {
            BossId::HawkEye => 2 * 3600,         // 2 hours = 7200s
            BossId::Roger => 90 * 60,            // 1.5 hours = 5400s
            BossId::SoulKing => 3600,            // 1 hour = 3600s
            BossId::RadiantAdmiral => 30 * 60,   // 30 minutes = 1800s
            BossId::TravellingMerchant => 30 * 60,// 30 minutes = 1800s
        }
    }

    pub fn cycle_display(&self) -> &'static str {
        match self {
            BossId::HawkEye => "Every 2h",
            BossId::Roger => "Every 1h 30m",
            BossId::SoulKing => "Every 1h",
            BossId::RadiantAdmiral => "Every 30m",
            BossId::TravellingMerchant => "Every 30m",
        }
    }

    pub fn despawn_info(&self) -> &'static str {
        match self {
            BossId::HawkEye => "Despawns in 30m",
            BossId::Roger => "Despawns in 30m",
            BossId::SoulKing => "Departs in 30m",
            BossId::RadiantAdmiral => "Despawns in 30m",
            BossId::TravellingMerchant => "Leaves in 10m",
        }
    }

    /// Calculates next spawn based on official Wiki UTC schedule.
    /// - Hawk Eye: UTC even hours (UTC+3 odd hours: 01:00, 03:00, etc.)
    /// - Roger: UTC modulo 5400 == 0 (UTC+3: 00:00, 01:30, 03:00, etc.)
    /// - Soul King: Every 1 hour (:00)
    /// - Radiant Admiral: Every 30 mins (:00, :30)
    /// - Travelling Merchant: Every 30 mins
    pub fn default_next_spawn_ts(&self, now: i64) -> i64 {
        let cycle = self.cycle_seconds();
        let past = now % cycle;
        if past == 0 {
            now
        } else {
            now + (cycle - past)
        }
    }
}

pub fn now_sec() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn format_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "Spawning now!".into();
    }
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;

    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Parses strings like "1h 13m and 23s", "1h 13m 23s", "13m and 13s", "13m13s", "45m", "90s", "1:13:23"
pub fn parse_duration_str(raw: &str) -> Option<i64> {
    let text = raw.trim().to_lowercase().replace("and", " ");
    if text.is_empty() {
        return None;
    }

    // Try format like "1:13:23" or "13:23"
    if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() == 3 {
            if let (Ok(h), Ok(m), Ok(s)) = (parts[0].trim().parse::<i64>(), parts[1].trim().parse::<i64>(), parts[2].trim().parse::<i64>()) {
                return Some(h * 3600 + m * 60 + s);
            }
        } else if parts.len() == 2 {
            if let (Ok(m), Ok(s)) = (parts[0].trim().parse::<i64>(), parts[1].trim().parse::<i64>()) {
                return Some(m * 60 + s);
            }
        }
    }

    let mut total_sec: i64 = 0;
    let mut curr_num = String::new();

    for ch in text.chars() {
        if ch.is_ascii_digit() {
            curr_num.push(ch);
        } else if ch == 'h' {
            if let Ok(n) = curr_num.parse::<i64>() {
                total_sec += n * 3600;
            }
            curr_num.clear();
        } else if ch == 'm' {
            if let Ok(n) = curr_num.parse::<i64>() {
                total_sec += n * 60;
            }
            curr_num.clear();
        } else if ch == 's' {
            if let Ok(n) = curr_num.parse::<i64>() {
                total_sec += n;
            }
            curr_num.clear();
        }
    }

    if total_sec > 0 {
        Some(total_sec)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertType {
    Warning5m,
    Spawned,
}

#[derive(Debug, Clone)]
pub struct BossAlert {
    pub boss: BossId,
    pub alert_type: AlertType,
    pub remaining_sec: i64,
}

pub struct BossTracker {
    pub offsets: HashMap<BossId, i64>,
    pub last_5m_alert: HashMap<BossId, i64>,
    pub last_spawn_alert: HashMap<BossId, i64>,
}

impl Default for BossTracker {
    fn default() -> Self {
        Self {
            offsets: HashMap::new(),
            last_5m_alert: HashMap::new(),
            last_spawn_alert: HashMap::new(),
        }
    }
}

impl BossTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_offset(&mut self, boss: BossId, target_ts: i64) {
        self.offsets.insert(boss, target_ts);
    }

    pub fn reset_offsets(&mut self) {
        self.offsets.clear();
    }

    /// Calculates next spawn timestamp for given boss.
    pub fn next_spawn_ts(&self, boss: BossId, now: i64) -> i64 {
        let cycle = boss.cycle_seconds();
        if let Some(&anchor) = self.offsets.get(&boss) {
            if anchor > now {
                anchor
            } else {
                let passed = now - anchor;
                let next = anchor + ((passed / cycle) + 1) * cycle;
                next
            }
        } else {
            boss.default_next_spawn_ts(now)
        }
    }

    pub fn remaining_seconds(&self, boss: BossId, now: i64) -> i64 {
        let target = self.next_spawn_ts(boss, now);
        target.saturating_sub(now)
    }

    /// Evaluates if any boss needs a 5-minute warning or spawn notification.
    pub fn tick(&mut self, now: i64, notify_5m: bool, notify_spawn: bool) -> Vec<BossAlert> {
        let mut alerts = Vec::new();

        for &boss in BossId::all() {
            let next_spawn = self.next_spawn_ts(boss, now);
            let rem = next_spawn.saturating_sub(now);

            // 5-Minute Warning: trigger when between 60s and 300s remaining
            if notify_5m && rem <= 300 && rem > 60 {
                let last_alerted = self.last_5m_alert.get(&boss).copied().unwrap_or(0);
                if last_alerted != next_spawn {
                    self.last_5m_alert.insert(boss, next_spawn);
                    alerts.push(BossAlert {
                        boss,
                        alert_type: AlertType::Warning5m,
                        remaining_sec: rem,
                    });
                }
            }

            // Spawn Alert: trigger when 0 <= rem <= 30
            if notify_spawn && rem <= 30 {
                let last_spawn_alerted = self.last_spawn_alert.get(&boss).copied().unwrap_or(0);
                if last_spawn_alerted != next_spawn {
                    self.last_spawn_alert.insert(boss, next_spawn);
                    alerts.push(BossAlert {
                        boss,
                        alert_type: AlertType::Spawned,
                        remaining_sec: rem,
                    });
                }
            }
        }

        alerts
    }

    /// Formats an HTML message summarizing all live boss countdowns for Telegram.
    pub fn format_status_message(&self, now: i64) -> String {
        let mut msg = String::from("👑 <b>GPO Event Bosses & Merchant Live Timers</b>\n\n");

        for &boss in BossId::all() {
            let rem = self.remaining_seconds(boss, now);
            let countdown = format_duration(rem);
            let is_imminent = rem <= 300;

            let icon = if is_imminent { "🚨" } else { "⏱" };
            msg.push_str(&format!(
                "{} <b>{}</b>\n   {} Next spawn: <b>{}</b> (<i>{}</i>)\n   📍 {}\n   ℹ️ {}\n\n",
                boss.emoji(),
                boss.name(),
                icon,
                countdown,
                boss.cycle_display(),
                boss.location(),
                boss.despawn_info(),
            ));
        }

        msg.push_str("💡 <i>Tip: Send <code>/sync</code> to calibrate with your in-game counter or Discord bot.</i>");
        msg
    }

    /// Parses pasted Discord message or sync string and returns sync result.
    pub fn parse_sync_text(&mut self, text: &str, now: i64) -> Result<String, String> {
        let lower = text.to_lowercase();

        // 1. Reset command
        if lower.contains("reset") {
            self.reset_offsets();
            return Ok("🔄 <b>Boss timers reset</b> to official GPO Wiki real-world schedule!".into());
        }

        let mut synced_bosses = Vec::new();

        // 2. Check if user pasted Discord message: "Event Bosses Live Spawn Times"
        if lower.contains("hawk eye") || lower.contains("roger") || lower.contains("soul king") || lower.contains("radiant admiral") || lower.contains("travelling merchant") || lower.contains("traveling merchant") {
            let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
            let mut current_boss: Option<BossId> = None;

            for line in lines {
                let l_low = line.to_lowercase();
                if l_low.contains("hawk eye") || l_low.contains("mihawk") {
                    current_boss = Some(BossId::HawkEye);
                } else if l_low.contains("roger") {
                    current_boss = Some(BossId::Roger);
                } else if l_low.contains("soul king") || l_low.contains("brook") {
                    current_boss = Some(BossId::SoulKing);
                } else if l_low.contains("radiant admiral") || l_low.contains("kizaru") {
                    current_boss = Some(BossId::RadiantAdmiral);
                } else if l_low.contains("merchant") {
                    current_boss = Some(BossId::TravellingMerchant);
                }

                if let Some(b) = current_boss {
                    if !l_low.contains("ago") && !l_low.contains("last spawn") && !l_low.contains("last refresh") {
                        if let Some(dur) = parse_duration_str(&l_low) {
                            self.set_offset(b, now + dur);
                            synced_bosses.push((b, dur));
                            current_boss = None;
                        }
                    }
                }
            }

            if !synced_bosses.is_empty() {
                let mut res = format!("✅ <b>Synced {} timers from Discord!</b>\n\n", synced_bosses.len());
                for (b, dur) in synced_bosses {
                    res.push_str(&format!("• {} {}: <b>{}</b> remaining\n", b.emoji(), b.name(), format_duration(dur)));
                }
                return Ok(res);
            }
        }

        // 3. Command: /sync <boss> <duration>
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() >= 2 {
            let target = parts[1].to_lowercase();
            let dur_str = parts[2..].join(" ");
            let target_boss = if target.contains("hawk") || target.contains("mihawk") {
                Some(BossId::HawkEye)
            } else if target.contains("roger") {
                Some(BossId::Roger)
            } else if target.contains("soul") || target.contains("king") || target.contains("brook") {
                Some(BossId::SoulKing)
            } else if target.contains("admiral") || target.contains("kizaru") || target.contains("radiant") {
                Some(BossId::RadiantAdmiral)
            } else if target.contains("merchant") {
                Some(BossId::TravellingMerchant)
            } else if target.contains("all") {
                // Quick-sync all: /sync all 1h13m23s 13m13s
                let remaining_parts: Vec<&str> = parts[2..].iter().copied().collect();
                if let Some(first_dur) = remaining_parts.first().and_then(|s| parse_duration_str(s)) {
                    // Hawk Eye (120m), Roger (90m), Soul King (60m)
                    self.set_offset(BossId::HawkEye, now + first_dur);
                    self.set_offset(BossId::Roger, now + first_dur);
                    self.set_offset(BossId::SoulKing, now + (first_dur % 3600));

                    if let Some(second_dur) = remaining_parts.get(1).and_then(|s| parse_duration_str(s)) {
                        self.set_offset(BossId::RadiantAdmiral, now + second_dur);
                        self.set_offset(BossId::TravellingMerchant, now + second_dur);
                    } else {
                        self.set_offset(BossId::RadiantAdmiral, now + (first_dur % 1800));
                        self.set_offset(BossId::TravellingMerchant, now + (first_dur % 1800));
                    }
                    return Ok("✅ <b>Synced all 5 boss & merchant timers!</b>".into());
                }
                None
            } else {
                None
            };

            if let Some(b) = target_boss {
                if let Some(dur) = parse_duration_str(&dur_str) {
                    self.set_offset(b, now + dur);
                    return Ok(format!("✅ Synced {} <b>{}</b> to <b>{}</b> remaining!", b.emoji(), b.name(), format_duration(dur)));
                } else {
                    return Err(format!("⚠️ Could not parse duration <code>{}</code>.\nExample: <code>/sync hawkeye 1h 13m 23s</code>", dur_str));
                }
            }
        }

        Err("ℹ️ <b>How to sync Boss Timers:</b>\n\n• <code>/sync hawkeye 1h 13m 23s</code>\n• <code>/sync roger 1h 13m 23s</code>\n• <code>/sync soulking 13m 23s</code>\n• <code>/sync admiral 13m 13s</code>\n• <code>/sync merchant 13m 13s</code>\n• <code>/sync all 1h13m23s 13m13s</code>\n• <code>/sync reset</code>\n\n💡 <i>Or simply copy and paste the entire Discord bot message here!</i>".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_strings() {
        assert_eq!(parse_duration_str("1h 13m and 23s"), Some(3600 + 13 * 60 + 23));
        assert_eq!(parse_duration_str("1h 13m 23s"), Some(3600 + 13 * 60 + 23));
        assert_eq!(parse_duration_str("13m and 13s"), Some(13 * 60 + 13));
        assert_eq!(parse_duration_str("13m13s"), Some(13 * 60 + 13));
        assert_eq!(parse_duration_str("45m"), Some(45 * 60));
        assert_eq!(parse_duration_str("1:13:23"), Some(3600 + 13 * 60 + 23));
        assert_eq!(parse_duration_str("13:23"), Some(13 * 60 + 23));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(3600 + 13 * 60 + 23), "1h 13m 23s");
        assert_eq!(format_duration(13 * 60 + 13), "13m 13s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(0), "Spawning now!");
    }

    #[test]
    fn test_default_wiki_schedules() {
        let now = 1789400000;
        for &boss in BossId::all() {
            let next = boss.default_next_spawn_ts(now);
            assert!(next >= now);
            assert!(next - now <= boss.cycle_seconds());
        }
    }

    #[test]
    fn test_parse_discord_message() {
        let mut tracker = BossTracker::new();
        let now = 100000;
        let discord_msg = "Event Bosses Live Spawn Times:\n• Hawk Eye\nTime left for next spawn:\n1h 13m and 23s\nLast spawn was: 46m and 36s ago\n\n• Roger\nTime left for next spawn:\n1h 13m and 23s\n\n• Soul King\nTime left for next spawn:\n13m and 23s\n\n• Radiant Admiral\nTime left for next spawn:\n13m and 13s\n\nTravelling Merchant Stock Refresh:\n• Travelling Merchant\nTime left for next refresh:\n13m and 13s";

        let res = tracker.parse_sync_text(discord_msg, now);
        assert!(res.is_ok());

        assert_eq!(tracker.remaining_seconds(BossId::HawkEye, now), 3600 + 13 * 60 + 23);
        assert_eq!(tracker.remaining_seconds(BossId::Roger, now), 3600 + 13 * 60 + 23);
        assert_eq!(tracker.remaining_seconds(BossId::SoulKing, now), 13 * 60 + 23);
        assert_eq!(tracker.remaining_seconds(BossId::RadiantAdmiral, now), 13 * 60 + 13);
        assert_eq!(tracker.remaining_seconds(BossId::TravellingMerchant, now), 13 * 60 + 13);
    }

    #[test]
    fn test_alerts_tick() {
        let mut tracker = BossTracker::new();
        let now = 100000;

        // Hawk eye at 5m (300s)
        tracker.set_offset(BossId::HawkEye, now + 300);
        // Radiant Admiral at 10s (spawn imminent)
        tracker.set_offset(BossId::RadiantAdmiral, now + 10);
        // Roger far away (1h)
        tracker.set_offset(BossId::Roger, now + 3600);

        let alerts = tracker.tick(now, true, true);
        assert_eq!(alerts.len(), 2);

        let hawkeye_alert = alerts.iter().find(|a| a.boss == BossId::HawkEye).unwrap();
        assert_eq!(hawkeye_alert.alert_type, AlertType::Warning5m);

        let admiral_alert = alerts.iter().find(|a| a.boss == BossId::RadiantAdmiral).unwrap();
        assert_eq!(admiral_alert.alert_type, AlertType::Spawned);

        // Subsequent tick within same window should not re-alert
        let alerts2 = tracker.tick(now + 1, true, true);
        assert_eq!(alerts2.len(), 0);
    }
}
