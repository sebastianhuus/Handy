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
you transcribed by hand.

## Model harness (Python / NeMo) -- for comparing candidate engines

Deliberately separate from `chunk_harness/` (Rust) until a decision is made
on whether/what to change there -- see `model_harness/README.md`. Heavy
dependency (torch + nemo_toolkit, several GB, its own venv), but produces
the same run-JSON schema, so everything below works on its output too.

```bash
cd research/long-form-transcription/model_harness
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

python3 run_engine.py --engine parakeet --input meeting.wav \
    --window-s 30 --overlap-s 6 --out run.json
# or: --engine canary --model nvidia/canary-1b-v2
```

## Evaluating a run (either harness) -- reproducible metrics, no notebook needed

```bash
cd research/long-form-transcription/eda
python3 evaluate.py --run-dir some_directory_of_run_jsons/ \
    --reference reference.txt --strip-fillers
```

Deterministic and dependency-free (stdlib only): same run JSON + reference
in, same table out, on any machine. Works whether the run JSON came from
`chunk_harness` or `model_harness`. `--reference` adds real WER (raw, and
after `postprocess.py`'s repeat-collapse filter); without it you still get
the proxy metrics from SPEC.md §6 (boundary confidence, words/audio-second,
throughput). See `eda/real_runs/FINDINGS.md` for what this has found so
far. The marimo notebook (`eda/notebooks/eda_long_form.py`) still works
too, for interactive sweeps against `fixtures/`.
