# Long-form transcription research

See [`SPEC.md`](SPEC.md) for the design and what's been validated so far.
This is the "how do I run it" quick start.

## EDA (Python / marimo) -- works right now, no model needed

Dependencies managed with [uv](https://docs.astral.sh/uv/) -- `uv sync`
creates `.venv/` and installs exactly what `uv.lock` pins (commit the lock
file; it's what makes this reproducible across machines, not the loose
version ranges in `pyproject.toml`).

```bash
cd research/long-form-transcription/eda
uv sync

# Unit tests for the merge algorithm, WER scoring, and evaluate.py (29 tests, no audio/model needed)
uv run pytest

# Regenerate the synthetic fixtures (deterministic; only needed if you edit
# fixtures/make_synthetic_fixtures.py)
uv run python3 fixtures/make_synthetic_fixtures.py

# Open the EDA notebook -- pick "fixtures" from the dropdown, no paths to type
uv run notebook
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
uv sync   # heavy: pulls torch + nemo_toolkit, several GB

uv run python3 run_engine.py --engine parakeet --input meeting.wav \
    --window-s 30 --overlap-s 6 --out run.json
# or: --engine canary --model nvidia/canary-1b-v2
```

## Dataset directory convention

Both `evaluate.py` and the notebook expect one directory per dataset,
directly under `eda/`, laid out like this (`fixtures/` and `real_runs/`
both already follow it):

```
eda/<dataset>/
    *.json            # run JSON from chunk_harness or model_harness
    reference.txt      # optional -- enables real WER
    summaries/          # evaluate.py's output: *.summary.json + merged text
```

## Evaluating a run -- reproducible metrics, no notebook needed

```bash
cd research/long-form-transcription/eda
uv run python3 evaluate.py --run-dir <dataset>/ \
    --reference <dataset>/reference.txt --strip-fillers --out-dir <dataset>/summaries/
```

Deterministic and dependency-free (stdlib only): same run JSON + reference
in, same table out, on any machine. Works whether the run JSON came from
`chunk_harness` or `model_harness`. `--reference` adds real WER (raw, and
after `postprocess.py`'s repeat-collapse filter); without it you still get
the proxy metrics from SPEC.md §6 (boundary confidence, words/audio-second,
throughput). `--out-dir` writes one `*.summary.json` per run (plus its
merged text) -- that's what the notebook below reads. In practice you
won't run this by hand often -- the notebook's "Regenerate summaries"
button does exactly this for whichever dataset you've selected.

## Browsing results (marimo notebook) -- loads pre-computed JSON, no recompute

```bash
cd research/long-form-transcription/eda
uv run notebook   # shortcut for: uv run marimo edit --no-token --watch notebooks/eda_long_form.py
```

Pick a dataset from the dropdown at the top -- every other path (run JSON,
`reference.txt`, `summaries/`) is derived from that one choice, nothing to
type. Opening the notebook never re-runs the merge algorithm or WER
scoring itself -- it only reads what `evaluate.py` already wrote (or, for
a brand-new dataset with no `summaries/` yet, tells you to click the
button first), so nobody pays that compute cost just to look at existing
results. Shows a sortable metrics table, bar charts (WER raw vs. after
repeat-collapse, merge-boundary confidence, throughput), and per-run
merged-text tabs. The **"Regenerate summaries for this dataset"** button
runs `evaluate.py` for you against the selected dataset when you've
produced new run JSON and want to (re-)score it -- that's the one action
in the notebook that costs anything, and it's opt-in.
