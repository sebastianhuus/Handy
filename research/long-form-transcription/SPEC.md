# Long-form / meeting-mode transcription: rolling-window research spec

Status: research, not yet an implementation plan for the app. Lives under
`research/` on `personal-features` only -- not a feature branch, not headed
upstream.

## 1. Problem

Handy's transcription pipeline (`managers/transcription.rs`) is built for
push-to-talk dictation: short utterances, one call to the loaded engine
(`engine.transcribe(...)` / `transcribe_with(...)`), done. Meeting-length
capture (recording continuously for 30 minutes to 2-3 hours) is a different
shape of problem:

- Feeding an entire multi-hour recording to Parakeet in one call is not
  something these ONNX ASR models are built for, and there's already a
  concrete data point that long single-pass audio is fragile even at very
  modest lengths: a closed bug against this app
  ([cjpais/Handy#1797](https://github.com/cjpais/Handy/issues/1797) for the
  feature request itself, and
  [cjpais/Handy#1464](https://github.com/cjpais/Handy/issues/1464) for a
  ~9-minute recording that failed to transcribe on Parakeet V2). Neither
  issue's root cause is confirmed fixed for the multi-hour case -- 9 minutes
  failing is the reason this document doesn't take "just run the whole file
  through Parakeet" as a viable option.
- The user's proposed fix: pass the model bounded windows of audio at a
  time ("large enough, possibly overlapping"), and stitch the per-window
  transcripts back into one coherent transcript. This document specs that
  approach and the tooling to validate it empirically before any of it
  touches the app.

## 2. Hypothesis under test

> Parakeet's output quality degrades on long single-pass audio (more so
> than short utterances), but a rolling window of overlapping chunks --
> transcribed independently and merged on the overlaps -- holds quality
> roughly flat regardless of total recording length, at the cost of some
> latency and boundary-splice complexity.

This is a hypothesis, not yet a finding. Nothing in this repo confirms it
against real audio -- see §7.

## 3. Design: rolling window

Two parameters:

- **window** (`W`): seconds of audio handed to Parakeet per call. Default
  guess: 30s.
- **overlap** (`O`): seconds shared between consecutive windows, so a
  sentence spoken across a window boundary still lands whole inside at
  least one window. Default guess: 6s (20% of window).
- **hop** = `W - O`: how far the window advances each step. Chunk `i`
  covers `[i*hop, i*hop + W]` in the original recording's timeline.

Each chunk is transcribed independently, with segment-level timestamps
(`TimestampGranularity::Segment` -- Parakeet supports this today, see
`chunk_harness`). Segment timestamps are re-based to the *original*
recording's timeline (chunk start + in-chunk offset), not chunk-relative --
every downstream step assumes this.

Open parameter question this research phase exists to answer: what values
of `W` and `O` hold quality flat as total duration grows into the hours
range, and what's the latency/compute cost of getting there (smaller `W`
= more calls = more model-load-amortized overhead; larger `O` = more
redundant compute, but more room for the merge step to find a confident
splice).

## 4. Merge (stitching) algorithm

Reference implementation: `eda/chunk_merge.py::merge_chunks`. Pure Python,
stdlib only, unit-tested (`eda/tests/test_chunk_merge.py`) independent of
any model or audio file -- this is the "structured tests" half of the ask,
and it already earned its keep once (see §7, the bug it caught).

For each adjacent chunk pair `(prev, next)`:

1. Compute the overlap window in original-recording time: `[next.start,
   prev.end]`.
2. Take `prev`'s segments whose midpoint falls in that window (`tail`) and
   `next`'s segments whose midpoint falls in it (`head`).
3. Word-tokenize both sides and run `difflib.SequenceMatcher` to find the
   longest common run of words.
4. If that run is `>= MIN_MATCH_WORDS` (3): splice there -- keep `tail`'s
   words up to the start of the match, then take `head`'s words from the
   start of the match onward (so the matched/duplicated words are kept
   exactly once, sourced from `head`). Mark the boundary confident.
5. Otherwise: the two chunks disagree on the overlap (real ASR variance, or
   a genuinely bad chunk). Fall back to a hard cut at the overlap's
   temporal midpoint -- `tail` keeps segments starting before the midpoint,
   `head` keeps segments starting at/after it. Mark the boundary
   `low_confidence`. **This path is lossy**: if the two sides don't
   overlap in *content* the way they overlap in *time* (e.g. one side
   mis-heard the entire window), the fallback can drop real words rather
   than duplicate them. The EDA notebook surfaces `low_confidence` boundary
   rate specifically so this failure mode is visible, not silently eaten.
6. Append whatever's left in `next` entirely beyond the overlap window.

Known limitation, documented rather than engineered around for this phase:
the splice always resolves to a single best-matching run and discards
anything the two sides disagree about before/after it within the overlap
window. A real implementation might want a proper alignment (à la
WhisperX's forced-alignment stitching) if `low_confidence` boundaries turn
out to be common on real audio -- that's a §8 follow-up, not in scope here.

## 5. Data format

The schema `chunk_harness` emits and `eda/chunk_merge.py` consumes:

```json
{
  "window_s": 30.0,
  "overlap_s": 6.0,
  "input": "meeting.wav",
  "chunks": [
    {
      "index": 0,
      "start": 0.0,
      "end": 30.0,
      "transcribe_seconds": 0.41,
      "segments": [
        {"start": 0.0, "end": 4.8, "text": "we should discuss the budget"}
      ]
    }
  ]
}
```

All timestamps are seconds, relative to the original recording (see §3).

## 6. Metrics to gather once real audio is available

- **WER** (`eda/wer.py`) of the merged transcript against a hand-checked
  reference, for at least a representative slice of a real recording. This
  is the only metric that actually answers the research question; every
  other metric here is a proxy for when a full reference isn't available.
- **Boundary confidence rate**: fraction of chunk-to-chunk splices that hit
  the `low_confidence` fallback. Ideally near 0 with a well-chosen `(W,
  O)`; rising with recording length would say overlap or window choice
  needs to scale with duration, not just be a fixed default.
- **Words-per-audio-second**: a cheap smell test per chunk and for the
  merged whole -- a chunk whose output rate is far from the recording's own
  average is a candidate for repetition/hallucination (a known Whisper-
  family failure mode on long or silent audio) independent of merging.
- **Per-chunk latency** (`transcribe_seconds` in the harness output): does
  quality-per-second-of-compute favor smaller windows (more calls, less
  redundant overlap compute) or larger ones (fewer calls, more context per
  call)?
- **Isolating merge-step error from model error**: run the same recording
  at `overlap_s = 0` (no merge splicing possible/needed) alongside the
  normal overlapping run. If WER is similar, the merge step isn't the
  problem; if overlapping is meaningfully better, that's direct evidence
  for the windowing approach; if larger windows alone (independent of
  overlap) already degrade WER, that's the Parakeet-degrades-on-length
  hypothesis confirmed and no amount of merge-algorithm tuning fixes it --
  the mitigation becomes "cap window length," not "chunk smarter."

## 7. What's implemented here vs. what still needs real audio

**Implemented and verified in this sandbox** (no model, no audio, no GPU
needed):

- `eda/chunk_merge.py` -- the merge algorithm, and `eda/wer.py` -- WER
  scoring. Both pure Python.
- `eda/tests/` -- 14 pytest unit tests covering: single/no chunks, no-
  time-overlap concatenation, confident-match splicing (exact duplicate
  overlap), the low-confidence fallback, an empty (VAD-silenced) chunk not
  crashing the pipeline, and the JSON schema round-trip. All pass.
  **These caught a real bug**: the first version of the confident-match
  splice dropped the entire duplicated overlap region instead of
  re-including it once from the `head` side -- invisible from the unit
  tests alone (I'd derived their expected strings by hand, from the same
  buggy reasoning as the code), but obvious the moment the synthetic
  fixture sweep below was run end-to-end and a *supposedly lossless* clean
  run came back with nonzero WER. That's the concrete case for having both
  layers: deterministic unit tests pin down intended behavior once you know
  what "correct" looks like, but a full round-trip sweep against a known
  reference is what actually catches "correct according to whom."
- `eda/fixtures/make_synthetic_fixtures.py` -- generates a fake multi-chunk
  run (no model) from a reference script laid onto a synthetic timeline,
  with an optional word-dropping "noise" mode to exercise both the
  confident and low-confidence merge paths. Current fixtures: a clean
  30s/6s run (0.0 WER after merge, confirming the splice is lossless when
  the two sides agree), and two noisy runs (30s/6s and 12s/2s) that
  demonstrate the low-confidence fallback is lossy by design when they
  don't.
- `eda/notebooks/eda_long_form.py` -- a marimo notebook that loads a
  directory of run JSON files, runs them through the merge algorithm,
  tables the metrics from §6, and lets you inspect the merged text per run.
  Runs clean end-to-end against the bundled fixtures (validated via
  `marimo export html` in this sandbox).
- `chunk_harness/` -- a standalone Rust CLI (not part of the app's Cargo
  workspace) that chunks a real WAV file and calls Parakeet per chunk,
  mirroring exactly how `managers/transcription.rs` invokes it
  (`ParakeetModel::load` + `ParakeetParams { timestamp_granularity:
  Some(TimestampGranularity::Segment) }` + `transcribe_with`). Its API
  usage was checked against `transcribe-rs`'s own source
  (`onnx/parakeet/mod.rs`, `lib.rs` on
  [cjpais/transcribe-rs](https://github.com/cjpais/transcribe-rs)), not
  guessed. **It has not been compiled** -- this sandbox's network egress
  blocks `static.crates.io`, so `cargo check` can't fetch dependencies
  here. Treat it as reviewed-against-source, not verified-by-compiler,
  until it's built once locally.

**Not implemented -- needs real audio and a real model, which this sandbox
doesn't have**:

- Any actual answer to §2's hypothesis. Nothing above touches a real
  Parakeet inference; the synthetic fixtures validate the *pipeline*
  mechanics, not model behavior.
- A hand-checked reference transcript for a real long recording (needed for
  real WER numbers).
- The `(W, O)` parameter sweep against real audio (§6).
- A decision on whether the merge algorithm's `MIN_MATCH_WORDS = 3`
  threshold and midpoint-cut fallback are good enough, or need the
  proper-alignment upgrade noted in §4.

## 8. Explicitly out of scope for this research phase

- Speaker diarization, system/loopback audio capture, storage format
  choice, history UI, mobile -- all raised in the original brainstorm, all
  downstream of "does chunked transcription actually work," which is what
  this phase answers first.
- Any change to `src-tauri/` / the shipped app. This is pre-implementation
  research; nothing here is wired into Handy.

## 9. How to run this

See `README.md` in this directory.
