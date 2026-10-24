pub mod actions;
pub mod crafting;
pub mod ctx;
pub mod machine;
pub mod ml_collect;
pub mod session;
pub mod trace;
pub mod watchdog;
pub mod telegram_remote;
pub mod web_server;
pub mod recorder;
pub mod shadow;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use parking_lot::{Mutex, RwLock};

use crate::config::{Settings, Store};
use crate::core::platform::Platform;
use crate::core::types::WindowInfo;
use crate::events::{BotEvent, BotState};
use crate::webhook::WebhookQueue;
use ctx::Ctx;

pub struct Bot {
    ctx: Arc<Ctx>,
    thread: Mutex<Option<JoinHandle<()>>>,
    watchdog: Mutex<Option<JoinHandle<()>>>,
    paused: AtomicBool,
    lifecycle: Mutex<()>,
    generation: AtomicU64,
}

fn join_unless_self(h: JoinHandle<()>) {
    if h.thread().id() != std::thread::current().id() {
        let _ = h.join();
    }
}

impl Bot {
    pub fn new(
        platform: Platform,
        settings: Arc<RwLock<Settings>>,
        roblox: Arc<RwLock<Option<WindowInfo>>>,
        events: Sender<BotEvent>,
        webhook: Arc<WebhookQueue>,
        store: Arc<Store>,
    ) -> Arc<Self> {
        let ctx = Arc::new(Ctx::new(platform, settings, roblox, events, webhook, store));
        Arc::new(Self {
            ctx,
            thread: Mutex::new(None),
            watchdog: Mutex::new(None),
            paused: AtomicBool::new(false),
            lifecycle: Mutex::new(()),
            generation: AtomicU64::new(0),
        })
    }

    pub fn prepare_update(self: &Arc<Self>, target: &str) -> Result<String, String> {
        let recording = recorder::get_status();
        if recording.is_playing || recording.is_recording || crafting::is_crafting_running() {
            return Err("Finish recording, playback or crafting before installing an update".into());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let was_running = self.is_running();
        let id = crate::core::secrets::generate_token_hex(16);
        let intent = crate::core::update_handoff::ResumeIntent {
            request_id:id.clone(), source:env!("CARGO_PKG_VERSION").into(),
            target:target.into(), created_ms:crate::events::now_ms(), was_running,
        };
        self.ctx.store.update_gate.lock().begin(intent)?;
        let mut quiesced = false;
        let result = (|| -> Result<(), String> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while !self.ctx.store.update_gate.lock().ready(&id)? {
                if !self.is_running() || std::time::Instant::now() >= deadline {
                    return Err("Update postponed: no safe reel boundary was reached; fishing state was preserved".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            let _lifecycle = self.lifecycle.lock();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("Update cancelled by a newer fishing control action".into());
            }
            self.ctx.store.update_gate.lock().ready(&id)?;
            self.ctx.input_epoch.fetch_add(1, Ordering::SeqCst);
            self.halt();
            quiesced = true;
            if was_running {
                self.paused.store(true, Ordering::SeqCst);
                self.ctx.session.lock().pause();
                self.ctx.set_state(BotState::Paused, Some("Installing update; fishing will resume after restart".into()));
                self.finalize_ml_session("update checkpoint");
            }
            self.ctx.store.save_stats(&self.ctx.session.lock().lifetime()).map_err(|e| e.to_string())?;
            self.ctx.store.save(&self.ctx.settings()).map_err(|e| e.to_string())?;
            self.ctx.store.commit_update_resume(&id)?;
            self.ctx.emit_stats();
            Ok(())
        })();
        if let Err(error) = result {
            // If preparation had already quiesced the session, recover it on
            // failure, provided a newer Stop/Pause did not revoke this request.
            let request = self.ctx.store.cancel_update_resume(Some(&id));
            if quiesced && request.is_some() && was_running && !self.is_running()
                && self.generation.load(Ordering::SeqCst) == generation
                && self.state() == BotState::Paused
            {
                let _ = self.start_at_generation(Some(generation));
            }
            return Err(error);
        }
        Ok(id)
    }

    pub fn cancel_update(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let generation = self.generation.load(Ordering::SeqCst);
        let request = self.ctx.store.cancel_update_resume(Some(id));
        if request.is_some_and(|(intent, prepared)| intent.was_running && prepared)
            && !self.is_running()
        {
            self.start_at_generation(Some(generation))?;
        }
        Ok(())
    }

    pub fn resume_after_update(self: &Arc<Self>) -> Result<(), String> {
        self.paused.store(true, Ordering::SeqCst);
        self.start_checked()
    }

    pub fn ctx(&self) -> &Arc<Ctx> {
        &self.ctx
    }

    pub fn is_running(&self) -> bool {
        self.ctx.running.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn state(&self) -> BotState {
        self.ctx.state()
    }

    pub fn start(self: &Arc<Self>) {
        if let Err(error) = self.start_checked() {
            self.ctx.log_error(&error);
            self.ctx.set_state(BotState::Stopped, Some(error));
        }
    }

    pub fn start_checked(self: &Arc<Self>) -> Result<(), String> {
        self.ctx.store.cancel_update_resume(None);
        self.start_at_generation(None)
    }

    fn start_at_generation(self: &Arc<Self>, expected: Option<u64>) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if expected.is_some_and(|g| self.generation.load(Ordering::SeqCst) != g) {
            return Ok(());
        }
        if self.is_running() {
            return Ok(());
        }
        if self.ctx.settings.read().fishing.trace {
            let preflight = crate::commands::collection_preflight(&self.ctx);
            if !preflight.ok {
                return Err(format!("Collection preflight failed: {}", preflight.checks.iter().filter(|c| !c.ok).map(|c| c.detail.as_str()).collect::<Vec<_>>().join("; ")));
            }
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.halt();
        let resume = self.paused.swap(false, Ordering::SeqCst);
        if resume {
            self.ctx.session.lock().resume();
        } else {
            let mut sess = self.ctx.session.lock();
            *sess = sess.next_session();
        }
        // Arm gameplay data collection for this macro run (normal start is
        // enough — no separate recording mode). Trace-gated downstream.
        // Ensure the dataset layout exists now so a broken path is loud in
        // the logs instead of silently yielding zero frames.
        if let Err(e) = self.ctx.ml.ensure_layout() {
            self.ctx.log_warn(&format!("ML DATA layout unavailable ({e}) — fishing without recording"));
        }
        let session_id = self.ctx.ml.begin_session(env!("CARGO_PKG_VERSION"));
        // Warm up shadow inference off the hot loop (observation-only
        // models load once into a process-global engine; first use after
        // this thread finishes pays nothing).
        if { let settings = self.ctx.settings.read(); settings.features.ml_shadow || settings.features.ml_fish_recognition } {
            let models_dir = self.ctx.store.dir().join("models");
            std::thread::spawn(move || {
                if crate::core::shadow_infer::engine(&models_dir).is_some() {
                    tracing::info!("local models warmed up; fish name recognition available");
                }
            });
        }
        self.emit_ml_session(None);
        self.ctx.emit_stats();
        self.spawn_loop(resume);
        self.spawn_watchdog();
        crate::laptop_fan::on_bot_state_changed(true);
        let trace_on = self.ctx.settings.read().fishing.trace;
        self.ctx.log_info(&if resume {
            format!(
                "Resumed (ML session {session_id} {})",
                if trace_on { "RECORDING" } else { "collection OFF — trace recording disabled" }
            )
        } else {
            format!(
                "Started (ML session {session_id} {})",
                if trace_on { "RECORDING" } else { "collection OFF — trace recording disabled" }
            )
        });
        Ok(())
    }

    pub fn pause(&self) {
        self.ctx.store.cancel_update_resume(None);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.ctx.input_epoch.fetch_add(1, Ordering::SeqCst);
        let was_running = self.ctx.running.swap(false, Ordering::SeqCst);
        self.ctx.platform.input.release_all();
        self.ctx.release_mouse();
        let _lifecycle = self.lifecycle.lock();
        if !was_running && self.state() != BotState::Recovering {
            return;
        }
        self.paused.store(true, Ordering::SeqCst);
        self.ctx.running.store(false, Ordering::SeqCst);
        self.ctx.set_state(BotState::Paused, None);
        self.ctx.log_info("Paused");
        self.halt();
        crate::laptop_fan::on_bot_state_changed(false);
        self.ctx.session.lock().pause();
        self.finalize_ml_session("paused");
        self.ctx.emit_stats();
    }

    pub fn stop(&self) {
        self.ctx.store.cancel_update_resume(None);
        crate::bot::crafting::stop_auto_craft();
        crate::bot::recorder::stop_playback();
        self.ctx.input_epoch.fetch_add(1, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.ctx.running.store(false, Ordering::SeqCst);
        self.ctx.platform.input.release_all();
        self.ctx.release_mouse();
        let _lifecycle = self.lifecycle.lock();
        let was = self.state() != BotState::Stopped || self.is_paused();
        self.paused.store(false, Ordering::SeqCst);
        self.ctx.running.store(false, Ordering::SeqCst);
        self.ctx.set_state(BotState::Stopped, None);
        if was {
            self.ctx.log_info("Stopped");
        }
        self.halt();
        crate::laptop_fan::on_bot_state_changed(false);
        self.ctx.session.lock().pause();
        // A stop can precede the loop's first state event. Close an active
        // collection shell even when the visible state still says Stopped.
        self.finalize_ml_session("stopped");
        self.ctx.emit_stats();
    }

    /// Change sample recording without interrupting fishing or inference.
    /// Serialize with start/stop so a disabled session cannot be rearmed by
    /// a concurrent start. Persist before changing live state.
    pub fn set_collection(&self, enabled: bool) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if enabled && self.is_running() {
            let preflight = crate::commands::collection_preflight(&self.ctx);
            let failures = preflight.checks.iter().filter(|c| c.name != "consent" && !c.ok).map(|c| c.detail.as_str()).collect::<Vec<_>>();
            if !failures.is_empty() { return Err(format!("Collection could not start: {}", failures.join("; "))); }
        }
        {
            let mut settings = self.ctx.settings.write();
            let mut next = settings.clone();
            next.fishing.trace = enabled;
            self.ctx.store.save(&next).map_err(|e| e.to_string())?;
            *settings = next;
        }
        if enabled {
            if self.is_running() && !self.ctx.ml.is_collecting() {
                self.ctx.ml.begin_session(env!("CARGO_PKG_VERSION"));
            }
            self.emit_ml_session(None);
        } else {
            self.finalize_ml_session("collection disabled");
            self.emit_ml_session(None);
        }
        self.ctx.log_info(if enabled { "Sample collection enabled; existing samples preserved" } else { "Sample collection stopped; fishing and name recognition continue" });
        Ok(())
    }

    /// Finalize the active collection session (flush, validate subset,
    /// summarize) and push it to the UI. No active session → silent no-op.
    fn finalize_ml_session(&self, why: &str) {
        if let Some(summary) = self.ctx.ml.end_session() {
            self.ctx.log_info(&format!(
                "ML DATA session complete: {} samples, {} reels, {} hard examples, quality {} ({} pending annotation, {} total)",
                summary.samples,
                summary.reels,
                summary.hard_examples,
                if summary.quality_ok { "PASS" } else { "WARNING" },
                summary.pending_annotation,
                summary.total_samples,
            ));
            for w in &summary.quality_warnings {
                self.ctx.log_warn(&format!("ML DATA quality: {w}"));
            }
            self.emit_ml_session(Some(&summary));
            let _ = why;
        }
    }

    fn emit_ml_session(&self, summary: Option<&crate::bot::ml_collect::SessionSummary>) {
        let snap = self.ctx.ml.stats_snapshot();
        // Honest collecting flag: a session shell alone (trace recording
        // OFF) must never present as recording — that exact lie let hours
        // of fishing pass with zero frames while the UI said COLLECTING.
        let trace_on = self.ctx.settings.read().fishing.trace;
        let st = crate::events::MlSessionState {
            // After finalization the live session is gone; the summary
            // carries the id (without this the UI could never match the
            // session-complete event to its session).
            collecting: summary.is_none() && snap.collecting && trace_on,
            session_id: summary
                .map(|s| s.session_id.clone())
                .or(snap.session_id),
            samples: snap.written,
            reels: snap.reels,
            hard_examples: snap.hard_examples,
            dropped: snap.dropped,
            quality_ok: summary.map(|s| s.quality_ok),
            quality_warnings: summary.map(|s| s.quality_warnings.clone()).unwrap_or_default(),
            pending_annotation: summary.map(|s| s.pending_annotation).unwrap_or(0),
            total_samples: summary.map(|s| s.total_samples).unwrap_or(0),
        };
        self.ctx.emit(BotEvent::MlSession(st));
    }

    pub fn toggle(self: &Arc<Self>) {
        if self.is_running() {
            self.pause();
        } else {
            self.start();
        }
    }

    pub fn recast(self: &Arc<Self>) {
        self.stop();
        let generation = self.generation.load(Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(400));
        if let Err(error) = self.start_at_generation(Some(generation)) { self.ctx.log_error(&error); }
        self.ctx.log_info("Remotely recasting rod");
    }

    fn halt(&self) {
        self.ctx.running.store(false, Ordering::SeqCst);
        self.ctx.platform.input.release_all();
        if let Some(h) = self.watchdog.lock().take() {
            join_unless_self(h);
        }
        if let Some(h) = self.thread.lock().take() {
            join_unless_self(h);
        }
        self.ctx.release_mouse();
    }

    fn spawn_loop(self: &Arc<Self>, skip_setup: bool) {
        self.ctx.running.store(true, Ordering::SeqCst);
        self.ctx.touch();
        let ctx = Arc::clone(&self.ctx);
        let weak = Arc::downgrade(self);
        let h = std::thread::Builder::new()
            .name("bot-loop".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    machine::run(&ctx, skip_setup);
                }));
                ctx.release_mouse();
                // All natural exits stop the watchdog and close collection. A
                // requested halt has already cleared running and owns cleanup.
                if ctx.running.swap(false, Ordering::SeqCst) {
                    if let Some(bot) = weak.upgrade() {
                        let paused = ctx.state() == BotState::Paused;
                        bot.paused.store(paused, Ordering::SeqCst);
                        if !paused { ctx.set_state(BotState::Stopped, None); }
                        ctx.session.lock().pause();
                        bot.finalize_ml_session("loop exited");
                        ctx.emit_stats();
                        crate::laptop_fan::on_bot_state_changed(false);
                    }
                }
                if result.is_err() {
                    ctx.log_error("Bot loop panicked; stopped");
                    ctx.running.store(false, Ordering::SeqCst);
                    ctx.set_state(BotState::Stopped, Some("panic".into()));
                }
            })
            .expect("spawn bot loop");
        *self.thread.lock() = Some(h);
    }

    fn spawn_watchdog(self: &Arc<Self>) {
        if !self.ctx.settings.read().watchdog.enabled {
            return;
        }
        let ctx = Arc::clone(&self.ctx);
        let weak: Weak<Bot> = Arc::downgrade(self);
        let on_stuck: Box<dyn FnOnce(String) + Send> = Box::new(move |reason| {
            if let Some(bot) = weak.upgrade() {
                bot.restart_loop(&reason);
            }
        });
        let h = std::thread::Builder::new()
            .name("bot-watchdog".into())
            .spawn(move || watchdog::run(&ctx, on_stuck))
            .expect("spawn watchdog");
        *self.watchdog.lock() = Some(h);
    }

    pub fn restart_loop(self: &Arc<Self>, reason: &str) {
        // A stop may be joining this watchdog. Never wait on its lifecycle lock.
        let Some(lifecycle) = self.lifecycle.try_lock() else { return };
        if !self.is_running() { return; }
        let generation = self.generation.load(Ordering::SeqCst);
        let attempt = {
            let mut s = self.ctx.session.lock();
            s.restarts += 1;
            s.restarts
        };
        let (max, backoff) = {
            let s = self.ctx.settings.read();
            (s.watchdog.max_restarts, s.watchdog.restart_backoff_s)
        };
        self.ctx.emit(BotEvent::Recovery { attempt, reason: reason.to_string() });
        self.ctx.webhook.recovery(attempt, reason);
        if attempt > max {
            self.ctx.log_error(&format!("Restart limit reached ({max}); stopping"));
            drop(lifecycle);
            self.stop();
            return;
        }
        self.ctx.log_warn(&format!("Restarting loop #{attempt}: {reason}"));
        self.ctx.set_state(BotState::Recovering, Some(reason.to_string()));
        self.ctx.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.watchdog.lock().take() {
            join_unless_self(h);
        }
        if let Some(h) = self.thread.lock().take() {
            join_unless_self(h);
        }
        self.ctx.release_mouse();
        drop(lifecycle);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(backoff);
        while std::time::Instant::now() < deadline {
            if self.generation.load(Ordering::SeqCst) != generation { return; }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _lifecycle = self.lifecycle.lock();
        if self.generation.load(Ordering::SeqCst) != generation { return; }
        self.spawn_loop(true);
        self.spawn_watchdog();
    }
}


#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn bot(tag: &str) -> Arc<Bot> {
        let dir = std::env::temp_dir().join(format!("gpo-lifecycle-{}-{}-{tag}", std::process::id(), crate::events::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut settings = Settings::default();
        settings.watchdog.enabled = false;
        settings.watchdog.restart_backoff_s = 0.3;
        settings.features.ml_shadow = false;
        let (tx, _) = crossbeam_channel::unbounded();
        Bot::new(crate::core::platform::mock::platform(), Arc::new(RwLock::new(settings)),
            Arc::new(RwLock::new(None)), tx, Arc::new(WebhookQueue::disabled()), Arc::new(Store::new(dir)))
    }
    fn wait_pending(bot: &Bot) {
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
        while !bot.ctx.store.update_gate.lock().pending() {
            assert!(std::time::Instant::now()<deadline,"update request did not start");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn update_waits_for_boundary_and_stop_revokes_resume() {
        let bot=bot("update-boundary");
        bot.ctx.running.store(true,Ordering::SeqCst);
        let worker=Arc::clone(&bot);
        let handle=std::thread::spawn(move || worker.prepare_update("99.0.0"));
        wait_pending(&bot);
        assert!(bot.is_running(),"download/preparation must not interrupt a reel");
        assert!(bot.ctx.store.update_boundary_reached());
        let id=handle.join().unwrap().unwrap();
        assert!(!bot.is_running());
        bot.stop();
        bot.cancel_update(&id).unwrap();
        assert!(!bot.is_running(),"Stop revokes any failure auto-resume");
        assert!(!bot.ctx.store.take_update_resume("99.0.0"));
        let _=std::fs::remove_dir_all(bot.ctx.store.dir());
    }
    #[test]
    fn paused_update_failure_never_starts_fishing() {
        let bot=bot("update-paused");
        let id=bot.prepare_update("99.0.0").unwrap();
        bot.cancel_update(&id).unwrap();
        assert!(!bot.is_running());
        let _=std::fs::remove_dir_all(bot.ctx.store.dir());
    }
    #[test]
    fn pause_during_reel_cancels_install_preparation() {
        let bot=bot("update-cancel");
        bot.ctx.running.store(true,Ordering::SeqCst);
        let worker=Arc::clone(&bot);
        let handle=std::thread::spawn(move || worker.prepare_update("99.0.0"));
        wait_pending(&bot);
        bot.pause();
        assert!(handle.join().unwrap().is_err());
        assert!(!bot.is_running());
        assert!(bot.is_paused());
        let _=std::fs::remove_dir_all(bot.ctx.store.dir());
    }
    #[test]
    fn simultaneous_starts_create_one_session() {
        let bot = bot("start");
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = (0..2).map(|_| {
            let bot = bot.clone(); let barrier = barrier.clone();
            std::thread::spawn(move || { barrier.wait(); bot.start_checked().unwrap(); })
        }).collect();
        barrier.wait();
        for thread in threads { thread.join().unwrap(); }
        assert!(bot.is_running());
        assert_eq!(bot.ctx.session.lock().lifetime().sessions, 1);
        bot.stop();
    }
    #[test]
    fn collection_can_stop_and_resume_without_stopping_fishing() {
        let bot = bot("collection-toggle");
        bot.start_checked().unwrap();
        bot.set_collection(true).unwrap();
        let first = bot.ctx.ml.active_session_id().unwrap();
        let sample = bot.ctx.store.dir().join("keep-this-sample.png");
        std::fs::write(&sample, b"existing sample sentinel").unwrap();
        bot.set_collection(false).unwrap();
        assert!(bot.is_running());
        assert!(!bot.ctx.settings.read().fishing.trace);
        assert!(!bot.ctx.ml.is_collecting());
        assert_eq!(std::fs::read(&sample).unwrap(), b"existing sample sentinel");
        bot.set_collection(true).unwrap();
        assert!(bot.is_running());
        assert!(bot.ctx.settings.read().fishing.trace);
        assert!(bot.ctx.ml.is_collecting());
        assert_ne!(bot.ctx.ml.active_session_id().unwrap(), first);
        bot.stop();
    }
    #[test]
    fn stop_during_backoff_cannot_restart() {
        let bot = bot("backoff");
        bot.start_checked().unwrap();
        let recovery = { let bot = bot.clone(); std::thread::spawn(move || bot.restart_loop("test")) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while bot.state() != BotState::Recovering && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(bot.state(), BotState::Recovering);
        bot.stop();
        recovery.join().unwrap();
        assert!(!bot.is_running());
        assert!(!bot.is_paused());
        assert_eq!(bot.state(), BotState::Stopped);
    }
}
