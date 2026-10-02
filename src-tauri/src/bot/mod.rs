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

use std::sync::atomic::{AtomicBool, Ordering};
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
        })
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
        if self.is_running() {
            return;
        }
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
    }

    pub fn pause(&self) {
        if !self.is_running() {
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
        let was = self.is_running() || self.is_paused();
        self.paused.store(false, Ordering::SeqCst);
        self.ctx.running.store(false, Ordering::SeqCst);
        self.ctx.set_state(BotState::Stopped, None);
        if was {
            self.ctx.log_info("Stopped");
        }
        self.halt();
        crate::laptop_fan::on_bot_state_changed(false);
        self.ctx.session.lock().pause();
        if was {
            self.finalize_ml_session("stopped");
        }
        self.ctx.emit_stats();
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
        std::thread::sleep(std::time::Duration::from_millis(400));
        self.start();
        self.ctx.log_info("Remotely recasting rod");
    }

    fn halt(&self) {
        self.ctx.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.watchdog.lock().take() {
            join_unless_self(h);
        }
        if let Some(h) = self.thread.lock().take() {
            join_unless_self(h);
        }
        self.ctx.release_mouse();
    }

    fn spawn_loop(&self, skip_setup: bool) {
        self.ctx.running.store(true, Ordering::SeqCst);
        self.ctx.touch();
        let ctx = Arc::clone(&self.ctx);
        let h = std::thread::Builder::new()
            .name("bot-loop".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    machine::run(&ctx, skip_setup);
                }));
                ctx.release_mouse();
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
        std::thread::sleep(std::time::Duration::from_secs_f32(backoff));
        self.spawn_loop(true);
        self.spawn_watchdog();
    }
}
