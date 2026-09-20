use std::io::Cursor;
use std::time::Duration;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::core::bait::BaitStock;
use crate::core::types::Frame;

pub fn scan_bait_stock_gemini(frame: &Frame, api_key: &str, model: &str) -> Result<BaitStock, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Gemini API key is empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    if frame.w == 0 || frame.h == 0 || frame.rgba.is_empty() {
        return Err("Empty frame provided for Gemini scan".into());
    }

    // Convert frame to PNG bytes in memory
    let img_buffer = image::RgbaImage::from_raw(frame.w as u32, frame.h as u32, frame.rgba.clone())
        .ok_or_else(|| "Failed to create image buffer from frame".to_string())?;

    let mut png_bytes = Vec::new();
    img_buffer
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode frame as PNG: {e}"))?;

    let b64_data = BASE64_STANDARD.encode(&png_bytes);

    let prompt = "Analyze this image from Grand Piece Online (GPO). First, check if the 'Fishing Baits' menu is actually open and visible. If the bait menu is NOT visible or the image shows only the game world/character/water, respond: {\"visible\": false}. If the bait menu IS visible, extract the exact numbers shown for each bait tier: Legendary Fish Bait, Rare Fish Bait, Common Fish Bait. Respond ONLY with valid JSON in this format: {\"visible\": true, \"legendary\": <number or null>, \"rare\": <number or null>, \"common\": <number or null>}";

    let payload = json!({
        "contents": [{
            "parts": [
                { "text": prompt },
                {
                    "inline_data": {
                        "mime_type": "image/png",
                        "data": b64_data
                    }
                }
            ]
        }],
        "generationConfig": {
            "response_mime_type": "application/json"
        }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Gemini API request failed: {e}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|e| format!("Failed to read Gemini response body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Gemini API error (status {status}): {body_text}"));
    }

    let parsed_res: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Gemini API response JSON: {e}"))?;

    let text = parsed_res
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("content"))
        .and_then(|cnt| cnt.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p0| p0.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("Invalid candidate text in Gemini response: {body_text}"))?;

    // Parse extracted JSON from text
    let clean_json = text.trim().strip_prefix("```json").unwrap_or(text.trim()).strip_suffix("```").unwrap_or(text.trim()).trim();

    let stock_obj: serde_json::Value = serde_json::from_str(clean_json)
        .map_err(|e| format!("Failed to parse bait stock JSON '{clean_json}': {e}"))?;

    let is_visible = stock_obj.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
    if !is_visible {
        return Err("Fishing Baits menu is not visible in frame".into());
    }

    let legendary = stock_obj.get("legendary").and_then(|v| v.as_u64()).map(|v| v as u32);
    let rare = stock_obj.get("rare").and_then(|v| v.as_u64()).map(|v| v as u32);
    let common = stock_obj.get("common").and_then(|v| v.as_u64()).map(|v| v as u32);

    Ok(BaitStock {
        legendary,
        rare,
        common,
    })
}

pub fn test_gemini_connection(api_key: &str, model: &str) -> Result<String, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key cannot be empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    let payload = json!({
        "contents": [{
            "parts": [
                { "text": "ping" }
            ]
        }]
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Connection failed: {e}"))?;

    let status = response.status();
    if status.is_success() {
        Ok(format!("Connected to {model_name} successfully!"))
    } else {
        let err_body = response.text().unwrap_or_default();
        Err(format!("Gemini API returned status {status}: {err_body}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FruitEventAnalysis {
    pub fruit_name: Option<String>,
    pub rarity: Option<String>,
    pub pity: Option<String>,
    pub event: String,
    pub reason: Option<String>,
    pub raw_text: String,
    #[serde(default)]
    pub telegram_message: Option<String>,
}

pub fn analyze_fruit_event_gemini(frame: &Frame, api_key: &str, model: &str) -> Result<FruitEventAnalysis, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Gemini API key is empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    if frame.w == 0 || frame.h == 0 || frame.rgba.is_empty() {
        return Err("Empty frame provided for Gemini scan".into());
    }

    let img_buffer = image::RgbaImage::from_raw(frame.w as u32, frame.h as u32, frame.rgba.clone())
        .ok_or_else(|| "Failed to create image buffer from frame".to_string())?;

    let mut png_bytes = Vec::new();
    img_buffer
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode frame as PNG: {e}"))?;

    let b64_data = BASE64_STANDARD.encode(&png_bytes);

    let prompt = "Look at this Grand Piece Online (GPO) Roblox screen notification banner. Extract all information about the devil fruit event.
Specifically:
1. Is it a fruit drop, a storage failure, or a fruit despawn notification?
2. What is the exact fruit name? (e.g. Kilo, Mochi, Pika, Tori, etc. If it just says 'Devil Fruit' without a specific name, set fruit_name to null).
3. What is the legendary pity count? (e.g. '27/100'). In GPO, pity is always out of 100.
4. What is the fruit rarity? (Common, Rare, Epic, Legendary, Mythical).
5. What is the reason or status description?
6. Write a clean, highly accurate, and attractive notification message in HTML format for Telegram (use <b>bold</b>, <i>italic</i>, and emojis). Include the exact fruit name, rarity tier, pity status, and action taken.
Respond ONLY with valid JSON in this format:
{
  \"fruit_name\": <string or null>,
  \"rarity\": <string or null>,
  \"pity\": <string or null>,
  \"event\": <\"drop\" | \"storage_full\" | \"dropped_ground\" | \"unknown\">,
  \"reason\": <string or null>,
  \"raw_text\": <string of all text on screen>,
  \"telegram_message\": <string of formatted HTML message or null>
}";

    let payload = json!({
        "contents": [{
            "parts": [
                { "text": prompt },
                {
                    "inline_data": {
                        "mime_type": "image/png",
                        "data": b64_data
                    }
                }
            ]
        }],
        "generationConfig": {
            "response_mime_type": "application/json"
        }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Gemini API request failed: {e}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|e| format!("Failed to read Gemini response body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Gemini API error (status {status}): {body_text}"));
    }

    let parsed_res: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Gemini API response JSON: {e}"))?;

    let text = parsed_res
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("content"))
        .and_then(|cnt| cnt.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p0| p0.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("Invalid candidate text in Gemini response: {body_text}"))?;

    let clean_json = text.trim().strip_prefix("```json").unwrap_or(text.trim()).strip_suffix("```").unwrap_or(text.trim()).trim();

    serde_json::from_str::<FruitEventAnalysis>(clean_json)
        .map_err(|e| format!("Failed to parse FruitEventAnalysis JSON '{clean_json}': {e}"))
}

pub fn rewrite_fruit_message_gemini(
    fruit_name: &str,
    rarity: &str,
    pity_info: &str,
    event_status: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Gemini API key is empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    let prompt = format!(
        "You are an assistant for the Roblox Grand Piece Online (GPO) Autofish macro.\n\
        Rewrite this devil fruit event into a clean, exciting, and accurate Telegram notification message.\n\
        Event details:\n\
        - Fruit Name: {fruit_name}\n\
        - Rarity: {rarity}\n\
        - Pity: {pity_info}\n\
        - Status: {event_status}\n\n\
        Format Requirements:\n\
        - Use Telegram HTML tags (<b>bold</b>, <i>italic</i>, <code>code</code>).\n\
        - Choose appropriate emojis based on rarity (🔥 for Mythical, 🌟 for Legendary, 🍇 for Rare/Common).\n\
        - Make it clear, exciting, and accurate.\n\
        - Output ONLY the raw Telegram HTML text without backticks, markdown codeblocks, or explanations."
    );

    let payload = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }]
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Gemini API request failed: {e}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|e| format!("Failed to read Gemini response body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Gemini API error (status {status}): {body_text}"));
    }

    let parsed_res: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Gemini API response JSON: {e}"))?;

    let text = parsed_res
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("content"))
        .and_then(|cnt| cnt.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p0| p0.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("Invalid candidate text in Gemini response: {body_text}"))?;

    let clean = text.trim().strip_prefix("```html").unwrap_or(text.trim()).strip_suffix("```").unwrap_or(text.trim()).trim();
    Ok(clean.to_string())
}

pub fn rewrite_spawn_message_gemini(
    fruit_name: Option<&str>,
    location: Option<&str>,
    is_ase: bool,
    raw_banner: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Gemini API key is empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    let f_str = fruit_name.unwrap_or("Unknown Devil Fruit");
    let loc_str = location.unwrap_or("Unknown Location");
    let ase_str = if is_ase { "Yes (All-Seeing Eye)" } else { "No" };

    let prompt = format!(
        "You are an assistant for the Roblox Grand Piece Online (GPO) Autofish macro.\n\
        A devil fruit has spawned in the game server! Format an exciting, clear, and attractive Telegram notification alert in HTML.\n\
        Details:\n\
        - Fruit Name: {f_str}\n\
        - Location: {loc_str}\n\
        - All-Seeing Eye (ASE): {ase_str}\n\
        - Raw Banner: {raw_banner}\n\n\
        Format Requirements:\n\
        - Use Telegram HTML tags (<b>bold</b>, <i>italic</i>, <code>code</code>).\n\
        - If All-Seeing Eye is detected, highlight it with 👁️ <b>ALL-SEEING EYE SPAWN ALERT!</b>\n\
        - Use appropriate fruit emojis (🔥 Mythical, 🌟 Legendary, 🍇 Rare, 🍎 Common).\n\
        - Include fruit name, rarity tier, location, and tip for the player.\n\
        - Output ONLY the raw Telegram HTML text without markdown code blocks, backticks, or extra commentary."
    );

    let payload = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }]
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Gemini API request failed: {e}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|e| format!("Failed to read Gemini response body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Gemini API error (status {status}): {body_text}"));
    }

    let parsed_res: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Gemini API response JSON: {e}"))?;

    let text = parsed_res
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("content"))
        .and_then(|cnt| cnt.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p0| p0.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("Invalid candidate text in Gemini response: {body_text}"))?;

    let clean = text.trim().strip_prefix("```html").unwrap_or(text.trim()).strip_suffix("```").unwrap_or(text.trim()).trim();
    Ok(clean.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartCommandResult {
    pub action: Option<String>,
    pub param: Option<u32>,
    pub reply: String,
}

pub fn interpret_smart_bot_command(
    user_query: &str,
    status_summary: &str,
    api_key: &str,
    model: &str,
) -> Result<SmartCommandResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Gemini API key is empty".into());
    }

    let model_name = if model.trim().is_empty() {
        "gemini-3.5-flash-lite"
    } else {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    };

    let prompt = format!(
        "You are the intelligent paired assistant for the Grand Piece Online (GPO) Autofish macro.\n\
        The user sent a message through Telegram: \"{user_query}\"\n\n\
        Current Macro Live Context:\n\
        {status_summary}\n\n\
        Available macro actions:\n\
        - \"start\": start or resume macro\n\
        - \"pause\": pause or stop macro\n\
        - \"recast\": recast the fishing rod\n\
        - \"buy_bait\": purchase fishing bait from merchant\n\
        - \"set_volume\": set master volume (param: 0 to 100)\n\
        - \"set_brightness\": set screen brightness (param: 0 to 100)\n\
        - \"screenshot\": take and send Roblox screenshot\n\
        - \"status\": send status report\n\
        - \"pity\": send devil fruit pity report\n\
        - \"bosses\": send boss countdowns\n\
        - \"web\": send web dashboard link\n\
        - null: if it's a general question or conversation\n\n\
        Instructions:\n\
        1. Determine if the user wants to trigger one of the actions above.\n\
        2. Write a clear, friendly, and helpful reply in Telegram HTML format (use <b>bold</b>, <i>italic</i>, and emojis).\n\
        3. If the user asked a question (e.g. pity, bosses, fishing performance, advice), answer it accurately using the live context provided.\n\
        Respond ONLY with valid JSON in this exact structure:\n\
        {{\n\
          \"action\": <string or null>,\n\
          \"param\": <number or null>,\n\
          \"reply\": <string of HTML response>\n\
        }}"
    );

    let payload = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }],
        "generationConfig": {
            "response_mime_type": "application/json"
        }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, key
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("Gemini API request failed: {e}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|e| format!("Failed to read Gemini response body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Gemini API error (status {status}): {body_text}"));
    }

    let parsed_res: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Gemini API response JSON: {e}"))?;

    let text = parsed_res
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("content"))
        .and_then(|cnt| cnt.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p0| p0.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("Invalid candidate text in Gemini response: {body_text}"))?;

    let clean = text.trim().strip_prefix("```json").unwrap_or(text.trim()).strip_suffix("```").unwrap_or(text.trim()).trim();
    serde_json::from_str::<SmartCommandResult>(clean)
        .map_err(|e| format!("Failed to parse SmartCommandResult '{clean}': {e}"))
}

