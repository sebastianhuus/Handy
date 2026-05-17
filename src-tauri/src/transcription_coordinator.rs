use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);

/// Minimum time a recording must run before stop is honored. Without this,
/// spamming a transcription hotkey starts and tears down CPAL mic streams
/// faster than the audio pipeline can handle, which has crashed the app.
/// Stops that arrive earlier block the coordinator thread for the remainder
/// of the window — subsequent events queue up and are processed afterwards.
const MIN_RECORDING_DURATION: Duration = Duration::from_millis(150);

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
    /// PTT recording: stops when the same PTT key is released.
    /// The stored binding_id is also what the audio manager started under.
    Recording {
        binding_id: String,
        started_at: Instant,
    },
    /// Toggle recording: stops on the next press of any transcription key.
    /// The stored binding_id is what the audio manager started under (may differ
    /// from the key the user pressed to start, e.g. after a PTT→toggle upgrade).
    RecordingToggle {
        binding_id: String,
        started_at: Instant,
    },
    Processing,
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
}

pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe" || id == "transcribe_with_post_process" || id == "transcribe_with_push_to_talk"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                            push_to_talk,
                        } => {
                            // Debounce rapid-fire press events (key repeat / double-tap).
                            // Releases always pass through for push-to-talk.
                            if is_pressed {
                                let now = Instant::now();
                                if last_press.map_or(false, |t| now.duration_since(t) < DEBOUNCE) {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press = Some(now);
                            }

                            if push_to_talk {
                                if is_pressed {
                                    match &stage {
                                        Stage::Idle => {
                                            start(&app, &mut stage, &binding_id, &hotkey_string, true);
                                        }
                                        // PTT pressed while a toggle (or upgraded PTT) recording is
                                        // active → stop that recording (cross-binding stop).
                                        Stage::RecordingToggle { binding_id: active_id, started_at } => {
                                            let active = active_id.clone();
                                            let started_at = *started_at;
                                            enforce_min_duration(started_at);
                                            stop(&app, &mut stage, &active, &hotkey_string);
                                        }
                                        _ => {
                                            debug!("Ignoring PTT press for '{binding_id}'");
                                        }
                                    }
                                } else if let Stage::Recording { binding_id: id, started_at } = &stage {
                                    // Key-up only stops PTT mode; RecordingToggle ignores key-up.
                                    if id == &binding_id {
                                        let started_at = *started_at;
                                        enforce_min_duration(started_at);
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                }
                            } else if is_pressed {
                                match &stage {
                                    Stage::Idle => {
                                        start(&app, &mut stage, &binding_id, &hotkey_string, false);
                                    }
                                    // Non-PTT binding fired while PTT is recording. handy-keys is
                                    // order-independent, so pressing the keys in either order fires
                                    // the longer combo. Upgrade to toggle mode without restarting
                                    // audio — the audio manager keeps the PTT binding_id.
                                    Stage::Recording { binding_id: ptt_id, started_at } => {
                                        let ptt = ptt_id.clone();
                                        let started_at = *started_at;
                                        debug!("PTT '{ptt}' upgraded to toggle mode by '{binding_id}'");
                                        stage = Stage::RecordingToggle { binding_id: ptt, started_at };
                                    }
                                    Stage::RecordingToggle { binding_id: active_id, started_at } if active_id == &binding_id => {
                                        let started_at = *started_at;
                                        enforce_min_duration(started_at);
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    // Different toggle binding pressed while recording → cross-binding stop.
                                    Stage::RecordingToggle { binding_id: active_id, started_at } => {
                                        let active = active_id.clone();
                                        let started_at = *started_at;
                                        enforce_min_duration(started_at);
                                        stop(&app, &mut stage, &active, &hotkey_string);
                                    }
                                    _ => {
                                        debug!("Ignoring press for '{binding_id}': pipeline busy")
                                    }
                                }
                            }
                        }
                        Command::Cancel {
                            recording_was_active,
                        } => {
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

#[cfg(test)]
mod tests {
    use super::is_transcribe_binding;

    #[test]
    fn is_transcribe_binding_recognises_plain_transcribe() {
        assert!(is_transcribe_binding("transcribe"));
    }

    #[test]
    fn is_transcribe_binding_recognises_post_process() {
        assert!(is_transcribe_binding("transcribe_with_post_process"));
    }

    #[test]
    fn is_transcribe_binding_recognises_push_to_talk() {
        assert!(
            is_transcribe_binding("transcribe_with_push_to_talk"),
            "transcribe_with_push_to_talk must be recognised as a transcribe binding"
        );
    }

    #[test]
    fn is_transcribe_binding_rejects_cancel() {
        assert!(!is_transcribe_binding("cancel"));
    }

    #[test]
    fn is_transcribe_binding_rejects_unknown() {
        assert!(!is_transcribe_binding("unknown_action"));
    }

    #[test]
    fn is_transcribe_binding_rejects_empty_string() {
        assert!(!is_transcribe_binding(""));
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
        .map_or(false, |a| a.is_recording())
    {
        let started_at = Instant::now();
        let binding_id = binding_id.to_string();
        *stage = if is_ptt {
            Stage::Recording { binding_id, started_at }
        } else {
            Stage::RecordingToggle { binding_id, started_at }
        };
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
}

/// Block the coordinator thread until the active recording has run for at
/// least `MIN_RECORDING_DURATION`. Subsequent key events queue in the mpsc
/// channel and are processed after the sleep; most will be dropped because
/// the stage transitions to `Processing` immediately after.
fn enforce_min_duration(started_at: Instant) {
    let elapsed = started_at.elapsed();
    if elapsed < MIN_RECORDING_DURATION {
        let remaining = MIN_RECORDING_DURATION - elapsed;
        debug!("Recording too short ({elapsed:?}); waiting {remaining:?} before stop");
        thread::sleep(remaining);
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
