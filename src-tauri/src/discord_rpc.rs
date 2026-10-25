use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use parking_lot::RwLock;
use serde_json::json;

use crate::bot::Bot;
use crate::config::Settings;
use crate::events::BotState;

const DISCORD_CLIENT_ID: &str = "1284163457199411200";

pub fn spawn(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || run_rpc_worker(bot, settings))
        .expect("spawn discord rpc worker");
}

fn run_rpc_worker(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    let pid = std::process::id();

    loop {
        // Check if Discord RPC is enabled
        let enabled = {
            let s = settings.read();
            s.features.discord_rpc
        };

        if !enabled {
            std::thread::sleep(Duration::from_secs(4));
            continue;
        }

        // Try to connect to Discord IPC pipe
        let mut pipe = match connect_discord_pipe() {
            Some(p) => p,
            None => {
                std::thread::sleep(Duration::from_secs(8));
                continue;
            }
        };

        // Handshake (opcode 0)
        let handshake = json!({
            "v": 1,
            "client_id": DISCORD_CLIENT_ID
        });
        if send_packet(&mut pipe, 0, &handshake).is_err() {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }

        // Read handshake response
        let _ = read_packet(&mut pipe);
        tracing::info!("Discord Rich Presence connected successfully");

        let mut nonce = 0u64;

        // Loop sending presence updates
        loop {
            let enabled = {
                let s = settings.read();
                s.features.discord_rpc
            };

            if !enabled {
                // Clear activity and break to outer reconnect loop
                let clear_payload = json!({
                    "cmd": "SET_ACTIVITY",
                    "args": {
                        "pid": pid,
                        "activity": null
                    },
                    "nonce": format!("{nonce}")
                });
                let _ = send_packet(&mut pipe, 1, &clear_payload);
                break;
            }

            nonce += 1;
            let running = bot.is_running();
            let paused = bot.is_paused();
            let state = bot.state();
            let stats = bot.ctx().session.lock().stats();

            let (state_text, details_text, start_time) = if !running && !paused {
                ("In Menu".to_string(), "Macro Idle".to_string(), None)
            } else {
                let status_icon = if paused {
                    "⏸️ Paused"
                } else {
                    match state {
                        BotState::Tracking => "🎣 Tracking Catch Bar",
                        BotState::WaitingForBite => "🐟 Waiting for Bite",
                        BotState::Casting => "🎯 Casting Line",
                        BotState::StoringFruit => "🍇 Storing Devil Fruit",
                        BotState::Purchasing => "🛒 Buying Bait",
                        _ => "🎣 Fishing in GPO",
                    }
                };

                let pity = stats.pity_fruit;
                let details = format!(
                    "🐟 {} Fish · 🍇 {} Fruits · ⚡ Pity: {}",
                    stats.fish, stats.fruits, pity
                );

                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let start = now_s.saturating_sub(stats.runtime_s as u64);

                (status_icon.to_string(), details, Some(start))
            };

            let mut activity = json!({
                "state": state_text,
                "details": details_text,
                "assets": {
                    "large_image": "gpo_icon",
                    "large_text": "GPO Autofish v4.0.3"
                }
            });

            if let Some(st) = start_time {
                activity["timestamps"] = json!({
                    "start": st
                });
            }

            let payload = json!({
                "cmd": "SET_ACTIVITY",
                "args": {
                    "pid": pid,
                    "activity": activity
                },
                "nonce": format!("{nonce}")
            });

            if send_packet(&mut pipe, 1, &payload).is_err() {
                tracing::warn!("Discord RPC connection lost, will reconnect");
                break;
            }

            // Drain responses from pipe
            let _ = read_packet(&mut pipe);

            std::thread::sleep(Duration::from_secs(5));
        }

        std::thread::sleep(Duration::from_secs(4));
    }
}

fn connect_discord_pipe() -> Option<File> {
    for i in 0..10 {
        let pipe_path = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(file) = OpenOptions::new().read(true).write(true).open(&pipe_path) {
            return Some(file);
        }
    }
    None
}

fn send_packet(pipe: &mut File, opcode: u32, payload: &serde_json::Value) -> Result<(), std::io::Error> {
    let json_bytes = payload.to_string().into_bytes();
    let len = json_bytes.len() as u32;

    pipe.write_all(&opcode.to_le_bytes())?;
    pipe.write_all(&len.to_le_bytes())?;
    pipe.write_all(&json_bytes)?;
    pipe.flush()?;
    Ok(())
}

fn read_packet(pipe: &mut File) -> Result<(u32, Vec<u8>), std::io::Error> {
    let mut header = [0u8; 8];
    pipe.read_exact(&mut header)?;

    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;

    let mut buf = vec![0u8; len];
    pipe.read_exact(&mut buf)?;
    Ok((opcode, buf))
}
