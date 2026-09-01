# Long-form transcription research

See [`SPEC.md`](SPEC.md) for the design and what's been validated so far.
This is the "how do I run it" quick start.

## EDA (Python / marimo) -- works right now, no model needed

```bash
cd research/long-form-transcription/eda
pip install -r requirements.txt

# Unit tests for the merge algorithm and WER scoring (14 tests, no audio/model needed)
pytest

# Regenerate the synthetic fixtures (deterministic; only needed if you edit
# fixtures/make_synthetic_fixtures.py)
python3 fixtures/make_synthetic_fixtures.py

# Open the EDA notebook -- loads fixtures/ by default
marimo edit notebooks/eda_long_form.py
```

## Chunk harness (Rust / real Parakeet) -- needs a downloaded model + real audio

Not part of the app's build -- standalone crate, doesn't touch
`src-tauri/`.

```bash
cd research/long-form-transcription/chunk_harness
cargo build --release
```

> This has been checked against `transcribe-rs`'s own source, not compiled
> in the sandbox that produced it (crates.io was unreachable there). Expect
> to fix minor build errors on first real build -- see SPEC.md §7.

Point `--model-dir` at a Parakeet model directory Handy has already
downloaded (app id is `com.pais.handy` -- on Linux that's under
`~/.local/share/com.pais.handy/models/`, macOS under `~/Library/Application
Support/com.pais.handy/models/`, Windows under
`%APPDATA%\com.pais.handy\models\`; look for a `parakeet-*` subdirectory),
and `--input` at a 16kHz mono WAV:

```bash
# Handy records/expects 16kHz mono; convert anything else first:
ffmpeg -i meeting.m4a -ac 1 -ar 16000 meeting.wav

./target/release/chunk_harness \
  --model-dir ~/.local/share/com.pais.handy/models/parakeet-tdt-0.6b-v3-int8 \
  --input meeting.wav \
  --window-s 30 --overlap-s 6 \
  --out ../eda/fixtures/real_w30_o6.json
```

Run it again at a few other `--window-s`/`--overlap-s` values (and at
`--overlap-s` just under `--window-s` for a control with effectively no
merge-splicing needed) to build up a sweep, then drop a hand-checked
`reference.txt` (plain text, next to the run JSON files) for whatever slice
you transcribed by hand, and point the notebook's `run_dir` field at that
directory instead of `fixtures/`.
