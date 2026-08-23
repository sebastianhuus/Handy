use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);
const RELEASE_GRACE: Duration = Duration::from_millis(50);

/// Minimum time a recording must run before a stop is honored. Without this,
/// spamming a transcription hotkey starts and tears down CPAL mic streams
/// faster than the audio pipeline can safely turn around, which has crashed
/// the app. A stop requested earlier blocks the coordinator thread for the
/// remainder of the window; events that queue up during the sleep are
/// processed afterwards (and mostly dropped, since the stage has already
/// moved to `Processing` by then).
const MIN_RECORDING_DURATION: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PttAction {
    Passthrough,
    DeferRelease,
    CancelRelease,
}

struct PendingRelease {
    binding_id: String,
    hotkey_string: String,
    deadline: Instant,
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
        push_to_talk: bool,
    },
    Cancel {
        recording_was_active: bool,
    },
    ProcessingFinished,
}

/// Pipeline lifecycle, owned exclusively by the coordinator thread.
enum Stage {
    Idle,
    /// Push-to-talk recording: only a release of the same `binding_id`
    /// stops it. `started_at` feeds the `MIN_RECORDING_DURATION` guard.
    Recording {
        binding_id: String,
        started_at: Instant,
    },
    /// Toggle recording — either started directly in toggle mode, or a PTT
    /// recording that got upgraded by a same-order/any-order chord press
    /// from another transcribe binding (see `classify_press`). Stops on the
    /// next press of *any* transcribe binding, never on release. The stored
    /// `binding_id` is always the one the audio manager actually started
    /// under, which `AudioRecordingManager::stop_recording` requires to
    /// match exactly.
    RecordingToggle {
        binding_id: String,
        started_at: Instant,
    },
    Processing,
}

/// Outcome of a transcribe-binding *press* event, decided from the current
/// recording stage. Pure and AppHandle-free so it's unit testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressOutcome {
    /// Nothing is recording — begin a new recording under this binding.
    Start,
    /// A PTT recording from a *different* binding is in progress. handy-keys
    /// dispatches chorded bindings order-independently, so pressing the
    /// keys in either order can fire the shorter binding first and the
    /// longer one second (or vice versa). Upgrade to toggle mode so the
    /// first binding's key-release no longer stops it — only an explicit
    /// stop press (from any transcribe binding) will now end it.
    UpgradeToToggle,
    /// A toggle recording (native or upgraded) is active. Any transcribe
    /// binding press — matching or not — stops it.
    CrossBindingStop,
    /// Pipeline busy (mid-processing) or a redundant same-binding PTT
    /// re-press while already recording; nothing to do.
    Ignore,
}

/// Decide what an `is_pressed` transcribe event should do.
///
/// `active` is `Some((active_binding_id, is_toggle))` describing the
/// in-progress recording, or `None` when idle. `busy` is true while the
/// pipeline is finishing a previous transcription and must ignore input.
fn classify_press(
    push_to_talk: bool,
    binding_id: &str,
    active: Option<(&str, bool)>,
    busy: bool,
) -> PressOutcome {
    if busy {
        return PressOutcome::Ignore;
    }

    match active {
        None => PressOutcome::Start,
        Some((_, true)) => PressOutcome::CrossBindingStop,
        Some((active_id, false)) => {
            if push_to_talk && active_id != binding_id {
                PressOutcome::UpgradeToToggle
            } else {
                PressOutcome::Ignore
            }
        }
    }
}

/// Block the coordinator thread until the active recording has run for at
/// least `MIN_RECORDING_DURATION`. See the constant's doc comment.
fn enforce_min_duration(started_at: Instant) {
    let elapsed = started_at.elapsed();
    if elapsed < MIN_RECORDING_DURATION {
        let remaining = MIN_RECORDING_DURATION - elapsed;
        debug!("Recording too short ({elapsed:?}); waiting {remaining:?} before stop");
        thread::sleep(remaining);
    }
}

fn classify_ptt_event(
    pending_release_binding: Option<&str>,
    is_pressed: bool,
    push_to_talk: bool,
    binding_id: &str,
    recording_binding: Option<&str>,
) -> PttAction {
    if !push_to_talk {
        return PttAction::Passthrough;
    }

    if is_pressed {
        if pending_release_binding == Some(binding_id) {
            PttAction::CancelRelease
        } else {
            PttAction::Passthrough
        }
    } else if recording_binding == Some(binding_id) && pending_release_binding.is_none() {
        PttAction::DeferRelease
    } else {
        PttAction::Passthrough
    }
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
}

pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe" || id == "transcribe_with_post_process"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                // Debounce is per-binding: a single global timestamp would also
                // suppress a *different* binding's press that fires within the
                // debounce window of the first, which happens routinely for a
                // fast any-order chord (e.g. fn then ctrl+fn a few ms apart).
                let mut last_press: HashMap<String, Instant> = HashMap::new();
                let mut pending_release: Option<PendingRelease> = None;

                loop {
                    let cmd = if let Some(pending) = &pending_release {
                        match rx.recv_timeout(
                            pending.deadline.saturating_duration_since(Instant::now()),
                        ) {
                            Ok(cmd) => cmd,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if let Some(pending) = pending_release.take() {
                                    if let Stage::Recording {
                                        binding_id: id,
                                        started_at,
                                    } = &stage
                                    {
                                        if id == &pending.binding_id {
                                            let started_at = *started_at;
                                            enforce_min_duration(started_at);
                                            stop(
                                                &app,
                                                &mut stage,
                                                &pending.binding_id,
                                                &pending.hotkey_string,
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match rx.recv() {
                            Ok(cmd) => cmd,
                            Err(_) => break,
                        }
                    };

                    match cmd {
                        Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                            push_to_talk,
                        } => {
                            let pending_release_binding = pending_release
                                .as_ref()
                                .map(|pending| pending.binding_id.as_str());
                            // Only a PTT `Recording` stage participates in the
                            // auto-repeat defer/cancel dance; `RecordingToggle`
                            // (native toggle, or a PTT recording upgraded by an
                            // any-order chord press) ignores key-ups entirely, so
                            // it must never report a `recording_binding` here.
                            let recording_binding = match &stage {
                                Stage::Recording { binding_id: id, .. } => Some(id.as_str()),
                                _ => None,
                            };

                            match classify_ptt_event(
                                pending_release_binding,
                                is_pressed,
                                push_to_talk,
                                &binding_id,
                                recording_binding,
                            ) {
                                PttAction::CancelRelease => {
                                    pending_release = None;
                                    continue;
                                }
                                PttAction::DeferRelease => {
                                    pending_release = Some(PendingRelease {
                                        binding_id,
                                        hotkey_string,
                                        deadline: Instant::now() + RELEASE_GRACE,
                                    });
                                    continue;
                                }
                                PttAction::Passthrough => {}
                            }

                            // Debounce rapid-fire press events (key repeat / double-tap)
                            // per binding. Push-to-talk releases may be deferred above
                            // to absorb X11 auto-repeat.
                            if is_pressed {
                                let now = Instant::now();
                                if last_press
                                    .get(&binding_id)
                                    .is_some_and(|t| now.duration_since(*t) < DEBOUNCE)
                                {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press.insert(binding_id.clone(), now);
                            }

                            if push_to_talk {
                                if is_pressed {
                                    let active = match &stage {
                                        Stage::Recording { binding_id: id, .. } => {
                                            Some((id.as_str(), false))
                                        }
                                        Stage::RecordingToggle { binding_id: id, .. } => {
                                            Some((id.as_str(), true))
                                        }
                                        _ => None,
                                    };
                                    let busy = matches!(stage, Stage::Processing);
                                    match classify_press(push_to_talk, &binding_id, active, busy) {
                                        PressOutcome::Start => {
                                            start(
                                                &app,
                                                &mut stage,
                                                &binding_id,
                                                &hotkey_string,
                                                true,
                                            );
                                        }
                                        PressOutcome::UpgradeToToggle => {
                                            if let Stage::Recording {
                                                binding_id: active_id,
                                                started_at,
                                            } = &stage
                                            {
                                                let active_id = active_id.clone();
                                                let started_at = *started_at;
                                                debug!(
                                                    "PTT '{active_id}' upgraded to toggle mode by '{binding_id}'"
                                                );
                                                stage = Stage::RecordingToggle {
                                                    binding_id: active_id,
                                                    started_at,
                                                };
                                            }
                                        }
                                        PressOutcome::CrossBindingStop => {
                                            if let Stage::RecordingToggle {
                                                binding_id: active_id,
                                                started_at,
                                            } = &stage
                                            {
                                                let active_id = active_id.clone();
                                                let started_at = *started_at;
                                                enforce_min_duration(started_at);
                                                stop(&app, &mut stage, &active_id, &hotkey_string);
                                            }
                                        }
                                        PressOutcome::Ignore => {
                                            debug!("Ignoring PTT press for '{binding_id}'");
                                        }
                                    }
                                } else if let Stage::Recording {
                                    binding_id: id,
                                    started_at,
                                } = &stage
                                {
                                    // Key-up only ever stops the matching PTT
                                    // binding; RecordingToggle ignores key-ups.
                                    if id == &binding_id {
                                        let started_at = *started_at;
                                        enforce_min_duration(started_at);
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                }
                            } else if is_pressed {
                                let active = match &stage {
                                    Stage::RecordingToggle { binding_id: id, .. } => {
                                        Some((id.as_str(), true))
                                    }
                                    Stage::Recording { binding_id: id, .. } => {
                                        Some((id.as_str(), false))
                                    }
                                    _ => None,
                                };
                                let busy = matches!(stage, Stage::Processing);
                                match classify_press(push_to_talk, &binding_id, active, busy) {
                                    PressOutcome::Start => {
                                        start(&app, &mut stage, &binding_id, &hotkey_string, false);
                                    }
                                    PressOutcome::CrossBindingStop => {
                                        if let Stage::RecordingToggle {
                                            binding_id: active_id,
                                            started_at,
                                        } = &stage
                                        {
                                            let active_id = active_id.clone();
                                            let started_at = *started_at;
                                            enforce_min_duration(started_at);
                                            stop(&app, &mut stage, &active_id, &hotkey_string);
                                        }
                                    }
                                    PressOutcome::UpgradeToToggle | PressOutcome::Ignore => {
                                        debug!("Ignoring press for '{binding_id}': pipeline busy");
                                    }
                                }
                            }
                        }
                        Command::Cancel {
                            recording_was_active,
                        } => {
                            pending_release = None;
                            // Don't reset during processing — wait for the pipeline to finish.
                            if !matches!(stage, Stage::Processing)
                                && (recording_was_active
                                    || matches!(
                                        stage,
                                        Stage::Recording { .. } | Stage::RecordingToggle { .. }
                                    ))
                            {
                                stage = Stage::Idle;
                            }
                        }
                        Command::ProcessingFinished => {
                            stage = Stage::Idle;
                        }
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self { tx }
    }

    /// Send a keyboard/signal input event for a transcribe binding.
    /// For signal-based toggles, use `is_pressed: true` and `push_to_talk: false`.
    pub fn send_input(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        push_to_talk: bool,
    ) {
        if self
            .tx
            .send(Command::Input {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                is_pressed,
                push_to_talk,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_cancel(&self, recording_was_active: bool) {
        if self
            .tx
            .send(Command::Cancel {
                recording_was_active,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }
}

fn start(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str, is_ptt: bool) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.start(app, binding_id, hotkey_string);
    if app
        .try_state::<Arc<AudioRecordingManager>>()
        .is_some_and(|a| a.is_recording())
    {
        let binding_id = binding_id.to_string();
        let started_at = Instant::now();
        *stage = if is_ptt {
            Stage::Recording {
                binding_id,
                started_at,
            }
        } else {
            Stage::RecordingToggle {
                binding_id,
                started_at,
            }
        };
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
}

fn stop(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.stop(app, binding_id, hotkey_string);
    *stage = Stage::Processing;
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // classify_press: any-order chord activation + cross-binding stop.
    //
    // Coverage for the sh-branch-2 (#5) port: PTT and toggle bindings that
    // share keys (e.g. "fn" and "ctrl+fn") must interact correctly
    // regardless of which one's hotkey fires first, and any transcribe
    // binding's press must be able to stop an active toggle recording.
    // -----------------------------------------------------------------

    const TRANSCRIBE: &str = "transcribe";
    const TRANSCRIBE_PP: &str = "transcribe_with_post_process";

    #[test]
    fn classify_press_starts_from_idle_in_either_mode() {
        assert_eq!(
            classify_press(true, TRANSCRIBE, None, false),
            PressOutcome::Start
        );
        assert_eq!(
            classify_press(false, TRANSCRIBE, None, false),
            PressOutcome::Start
        );
    }

    #[test]
    fn classify_press_ignores_everything_while_pipeline_busy() {
        assert_eq!(
            classify_press(true, TRANSCRIBE, None, true),
            PressOutcome::Ignore
        );
        assert_eq!(
            classify_press(false, TRANSCRIBE, Some((TRANSCRIBE, true)), true),
            PressOutcome::Ignore
        );
    }

    #[test]
    fn classify_press_ignores_redundant_same_binding_ptt_repress() {
        // Same PTT binding fires again while already recording (e.g. a
        // debounce-adjacent repeat) — its own release still stops it, so
        // there's nothing to do here.
        assert_eq!(
            classify_press(true, TRANSCRIBE, Some((TRANSCRIBE, false)), false),
            PressOutcome::Ignore
        );
    }

    #[test]
    fn classify_press_upgrades_ptt_to_toggle_regardless_of_chord_order() {
        // fn (transcribe) fires first as PTT; ctrl+fn (transcribe_with_post_process)
        // follows while it's still held.
        assert_eq!(
            classify_press(true, TRANSCRIBE_PP, Some((TRANSCRIBE, false)), false),
            PressOutcome::UpgradeToToggle
        );
        // The reverse physical order — ctrl+fn's binding fires first, then fn's —
        // must upgrade identically. Any-order means neither binding is "the"
        // canonical starter.
        assert_eq!(
            classify_press(true, TRANSCRIBE, Some((TRANSCRIBE_PP, false)), false),
            PressOutcome::UpgradeToToggle
        );
    }

    #[test]
    fn classify_press_does_not_upgrade_outside_push_to_talk() {
        // In toggle mode a recording is always native RecordingToggle
        // (is_toggle = true), never PTT `Recording`, so this combination
        // shouldn't arise in practice — but if it did, non-PTT sessions
        // must never upgrade (there's nothing to upgrade *from*).
        assert_eq!(
            classify_press(false, TRANSCRIBE_PP, Some((TRANSCRIBE, false)), false),
            PressOutcome::Ignore
        );
    }

    #[test]
    fn classify_press_cross_binding_stop_while_toggling() {
        // A different binding's press stops an active toggle recording...
        assert_eq!(
            classify_press(false, TRANSCRIBE_PP, Some((TRANSCRIBE, true)), false),
            PressOutcome::CrossBindingStop
        );
        // ...and so does the same binding pressed again (classic toggle-off).
        assert_eq!(
            classify_press(false, TRANSCRIBE, Some((TRANSCRIBE, true)), false),
            PressOutcome::CrossBindingStop
        );
    }

    #[test]
    fn classify_press_cross_binding_stop_after_ptt_upgrade() {
        // Once a PTT recording has been upgraded to toggle mode, ANY
        // transcribe binding press stops it — including a third binding
        // that was never involved in starting it.
        assert_eq!(
            classify_press(
                true,
                "some_other_transcribe_binding",
                Some((TRANSCRIBE_PP, true)),
                false
            ),
            PressOutcome::CrossBindingStop
        );
    }

    // -----------------------------------------------------------------
    // enforce_min_duration: MIN_RECORDING_DURATION guard.
    //
    // Spamming a transcription hotkey was crashing the app via rapid CPAL
    // mic stream open/close cycles; a stop requested too soon after start
    // must be deferred until the window elapses, and must not delay a stop
    // that already satisfies it.
    // -----------------------------------------------------------------

    #[test]
    fn enforce_min_duration_defers_a_stop_requested_immediately_after_start() {
        let started_at = Instant::now();
        let before = Instant::now();
        enforce_min_duration(started_at);
        let waited = before.elapsed();

        // Allow a little slack for scheduler jitter, but it must have
        // waited most of the window — proving the early stop was deferred
        // rather than honored immediately.
        assert!(
            waited >= MIN_RECORDING_DURATION.saturating_sub(Duration::from_millis(20)),
            "expected an early stop to be deferred close to {MIN_RECORDING_DURATION:?}, only waited {waited:?}"
        );
        // And that the deferred stop does eventually return (fire) rather
        // than block forever.
        assert!(
            waited < MIN_RECORDING_DURATION + Duration::from_millis(500),
            "deferred stop should fire shortly after the window elapses, waited {waited:?}"
        );
    }

    #[test]
    fn enforce_min_duration_does_not_delay_a_stop_after_window_elapsed() {
        let started_at = Instant::now() - MIN_RECORDING_DURATION - Duration::from_millis(50);
        let before = Instant::now();
        enforce_min_duration(started_at);
        let waited = before.elapsed();

        assert!(
            waited < Duration::from_millis(20),
            "a stop requested after MIN_RECORDING_DURATION already elapsed must not block, waited {waited:?}"
        );
    }

    #[test]
    fn push_to_talk_release_while_recording_defers_release() {
        assert_eq!(
            classify_ptt_event(None, false, true, "transcribe", Some("transcribe")),
            PttAction::DeferRelease
        );
    }

    #[test]
    fn push_to_talk_press_matching_pending_release_cancels_release() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::CancelRelease
        );
    }

    #[test]
    fn toggle_mode_press_and_release_pass_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                false,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
        assert_eq!(
            classify_ptt_event(None, false, false, "transcribe", Some("transcribe")),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_for_different_binding_than_pending_release_passes_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "transcribe_with_post_process",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_matching_pending_release_cancels_without_recording_state() {
        assert_eq!(
            classify_ptt_event(Some("transcribe"), true, true, "transcribe", None),
            PttAction::CancelRelease
        );
    }

    // ---------------------------------------------------------------------
    // Sequence-level regression coverage for issue #1539.
    //
    // Under X11 key auto-repeat, holding a push-to-talk key does not emit one
    // long press. It emits the initial press followed by a stream of
    // synthesized release/press pairs, then a single genuine release on key-up.
    // Before the fix, every synthesized release passed straight through and
    // stopped recording, so holding the key "rapidly toggled" recording on and
    // off. The fix defers each release for a short grace window and cancels it
    // when the matching auto-repeat press arrives.
    //
    // The unit tests above assert `classify_ptt_event` in isolation. The
    // simulator below threads that classifier through the same `pending_release`
    // / `stage` state transitions the coordinator loop performs (lines that
    // handle `Command::Input` and the `recv_timeout` grace expiry), so a whole
    // event burst can be exercised deterministically without a Tauri AppHandle
    // or real timers.
    // ---------------------------------------------------------------------

    const BINDING: &str = "transcribe";

    #[derive(Clone, Copy)]
    enum Ev {
        /// A key-down event (real initial press or a synthesized auto-repeat press).
        Press,
        /// A key-up event (synthesized auto-repeat release or the genuine key-up).
        Release,
        /// The `RELEASE_GRACE` window elapsed with no cancelling press arriving.
        Grace,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum SimStage {
        Idle,
        Recording,
        Processing,
    }

    struct SimResult {
        starts: u32,
        stops: u32,
        stage: SimStage,
    }

    /// Mirror of the coordinator loop's decision logic for a single push-to-talk
    /// binding: it calls the real `classify_ptt_event` and applies the exact same
    /// Defer / Cancel / debounce / start / stop transitions.
    fn simulate(events: &[Ev]) -> SimResult {
        let mut stage = SimStage::Idle;
        let mut pending: Option<String> = None;
        let mut last_press_ms: Option<u64> = None;
        let mut clock_ms: u64 = 0;
        let mut starts = 0u32;
        let mut stops = 0u32;
        let debounce_ms = DEBOUNCE.as_millis() as u64;

        for ev in events {
            // Auto-repeat events arrive a few ms apart, well inside DEBOUNCE.
            clock_ms += 5;

            match ev {
                Ev::Grace => {
                    // Coordinator's `RecvTimeoutError::Timeout` arm: fire the
                    // deferred release iff we are still recording that binding.
                    if let Some(pending_binding) = pending.take() {
                        if stage == SimStage::Recording && pending_binding == BINDING {
                            stage = SimStage::Processing;
                            stops += 1;
                        }
                    }
                }
                Ev::Press | Ev::Release => {
                    let is_pressed = matches!(ev, Ev::Press);
                    let pending_binding = pending.as_deref();
                    let recording_binding = if stage == SimStage::Recording {
                        Some(BINDING)
                    } else {
                        None
                    };

                    match classify_ptt_event(
                        pending_binding,
                        is_pressed,
                        true, // push_to_talk
                        BINDING,
                        recording_binding,
                    ) {
                        PttAction::CancelRelease => {
                            pending = None;
                            continue;
                        }
                        PttAction::DeferRelease => {
                            pending = Some(BINDING.to_string());
                            continue;
                        }
                        PttAction::Passthrough => {}
                    }

                    if is_pressed {
                        if last_press_ms.is_some_and(|t| clock_ms - t < debounce_ms) {
                            continue;
                        }
                        last_press_ms = Some(clock_ms);
                    }

                    if is_pressed && stage == SimStage::Idle {
                        stage = SimStage::Recording;
                        starts += 1;
                    } else if !is_pressed && stage == SimStage::Recording {
                        stage = SimStage::Processing;
                        stops += 1;
                    }
                }
            }
        }

        SimResult {
            starts,
            stops,
            stage,
        }
    }

    /// Initial press plus several synthesized release/press pairs, as X11 emits
    /// while a push-to-talk key is held down.
    fn autorepeat_burst() -> Vec<Ev> {
        let mut events = vec![Ev::Press];
        for _ in 0..6 {
            events.push(Ev::Release);
            events.push(Ev::Press);
        }
        events
    }

    /// Regression for #1539: a burst of X11 auto-repeat release/press pairs must
    /// not stop recording. Before the fix the first synthesized release stopped
    /// recording immediately (stops == 1, stage left Recording), which produced
    /// the rapid on/off toggling. With the fix the releases are coalesced and
    /// recording stays continuously active for the whole burst.
    #[test]
    fn x11_autorepeat_burst_does_not_toggle_recording() {
        let result = simulate(&autorepeat_burst());
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 0,
            "synthesized auto-repeat releases must not stop recording mid-burst"
        );
        assert_eq!(
            result.stage,
            SimStage::Recording,
            "recording must remain active across the entire auto-repeat burst"
        );
    }

    /// Complements the burst test: once the key is genuinely released and the
    /// grace window elapses with no re-press, recording stops exactly once. This
    /// proves the debounce only coalesces synthesized releases and does not wedge
    /// the coordinator or swallow the real key-up.
    #[test]
    fn genuine_release_after_grace_stops_recording_once() {
        let mut events = autorepeat_burst();
        events.push(Ev::Release); // genuine key-up
        events.push(Ev::Grace); // grace window elapses, no cancelling press
        let result = simulate(&events);
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 1,
            "a genuine release should stop recording exactly once"
        );
        assert_eq!(result.stage, SimStage::Processing);
    }
}
