# model_harness -- pure-Python engine comparison, decoupled from the app

Deliberately separate from `../chunk_harness` (Rust, `transcribe-rs`/ONNX --
what the app would actually ship) until a decision is made on whether/what
to change there. Emits the exact same run-JSON schema `chunk_harness` does
(plus two extra fields, `engine`/`model`/`segment_timestamps` -- ignored by
old tooling, used by `../eda/evaluate.py`), so every existing, already-
tested piece of this research (`chunk_merge.py`, `postprocess.py`,
`wer.py`, `evaluate.py`) works unchanged regardless of which harness
produced a given run file.

## Why NeMo, not transcribe-rs's ONNX port

Two reasons, found empirically (see `../SPEC.md` and
`../eda/real_runs/FINDINGS.md`):

1. Canary's ONNX port in `transcribe-rs` always returns `segments: None` --
   no per-segment timestamps at all, which `chunk_merge.py`'s
   overlap-splicing needs. NeMo's own Canary implementation does return
   real timestamps (`timestamps=True`); a bug report against this existed
   in NeMo as of Oct 2025 but did not reproduce here (Sept 2026) -- it
   appears fixed.
2. Comparing the ONNX-int8 path against NeMo's fp32 reference implementation
   *of the same model* isolated a real quality difference in the runtime,
   not just the model choice -- see FINDINGS.md's repetition-artifact
   writeup. Using the reference implementation here keeps that variable
   controlled when comparing candidate engines.

This is a heavy dependency (torch + nemo_toolkit, several GB, its own
venv) -- an accepted tradeoff for using each model's correctness-reference
implementation rather than reimplementing TDT/AED decoding by hand.

## Setup

```bash
cd research/long-form-transcription/model_harness
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt   # heavy: pulls torch + nemo_toolkit, several GB
```

## Usage

```bash
python3 run_engine.py --engine parakeet --model nvidia/parakeet-tdt-0.6b-v2 \
    --input meeting.wav --window-s 30 --overlap-s 6 --out run.json

python3 run_engine.py --engine canary --model nvidia/canary-1b-v2 \
    --input meeting.wav --window-s 30 --overlap-s 6 --out run.json
```

`--model` defaults to `nvidia/parakeet-tdt-0.6b-v2` / `nvidia/canary-1b-v2`
per engine if omitted -- pass a different HF model id (e.g.
`nvidia/parakeet-tdt-0.6b-v3`) or a local `.nemo` path to compare other
checkpoints. First run per model downloads the checkpoint from Hugging
Face (multi-GB) to `~/.cache/huggingface/hub/`.

`--device auto|cpu|mps|cuda` (default `auto`). CPU was used for every run
in this research for determinism/comparability across the sweep, not
because MPS didn't work -- try `--device mps` on Apple Silicon if you want
the speedup (`torch.backends.mps.is_available()` was `True` on the machine
this was built on; not otherwise validated here).

Then evaluate exactly like a `chunk_harness` run:

```bash
python3 ../eda/evaluate.py --run run.json --reference reference.txt --strip-fillers
```

## Determinism

Decoding is greedy for both engines here (no beam search, no sampling) --
same input produces the same output on the same machine. Floating-point
results can still differ in the last bit or two across different hardware
(CPU vs MPS, different CPU microarchitectures) or PyTorch/NeMo versions --
this harness does not attempt bit-exact cross-machine reproducibility,
only same-machine determinism and cross-machine result *comparability*
(the kind of small float noise here doesn't change word-level output in
practice). Pin the exact package versions in `requirements.txt`
(`pip freeze`) if stricter reproducibility across machines matters more
than staying on latest.

## What NOT to read into this

This harness exists to compare *engines*, not to prototype a shipping
Handy feature -- it writes a temp WAV file per chunk and shells out to
NeMo's high-level `.transcribe()` API, which is fine for research
throughput but not how the app would actually call a model. See
`../eda/real_runs/FINDINGS.md` for what the comparisons found and what, if
anything, that implies for `chunk_harness`/the app.
