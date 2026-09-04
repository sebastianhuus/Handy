#!/usr/bin/env python3
"""Pure-Python equivalent of chunk_harness (Rust), for candidate engines not
yet decided on for the app: NVIDIA Parakeet-TDT and NVIDIA Canary, both run
via NeMo -- the engines' own reference toolkit -- rather than transcribe-rs's
ONNX port. Deliberately decoupled from chunk_harness/ and the Rust app code;
see ../SPEC.md and ./README.md for why.

Emits the exact same run-JSON schema chunk_harness does, so the existing,
already-tested ../eda/chunk_merge.py, postprocess.py, and wer.py all work
unchanged regardless of which harness (this one or the Rust one) produced a
given run file:

    {
      "window_s": 30.0, "overlap_s": 6.0, "input": "meeting.wav",
      "engine": "parakeet", "model": "nvidia/parakeet-tdt-0.6b-v2",
      "segment_timestamps": "exact" | "estimated",
      "chunks": [
        {"index": 0, "start": 0.0, "end": 30.0, "transcribe_seconds": 0.8,
         "segments": [{"start": 0.0, "end": 4.8, "text": "..."}]}
      ]
    }

"segment_timestamps" is the one field chunk_harness's schema doesn't have --
"exact" means real per-segment start/end came back from the model (as
Parakeet's ONNX port already gives us); "estimated" means the engine
returned no usable timestamps for this run (Canary, as of NeMo's current
release -- see README.md) and each chunk's one segment spans the whole
chunk, un-timed. chunk_merge.py picks its splicing strategy from this field
explicitly rather than guessing from segment shape.

Usage:
    python3 run_engine.py --engine parakeet --input meeting.wav \\
        --window-s 30 --overlap-s 6 --out run.json
    python3 run_engine.py --engine canary --input meeting.wav \\
        --window-s 30 --overlap-s 6 --out run.json
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import time
from pathlib import Path

SAMPLE_RATE = 16_000

DEFAULT_MODELS = {
    "parakeet": "nvidia/parakeet-tdt-0.6b-v2",
    "canary": "nvidia/canary-1b-v2",
}


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--engine", required=True, choices=sorted(DEFAULT_MODELS))
    p.add_argument("--model", default=None, help=f"HF model id or local .nemo path (default: per-engine, see {DEFAULT_MODELS})")
    p.add_argument("--input", required=True, type=Path, help="16kHz mono PCM WAV")
    p.add_argument("--window-s", type=float, default=30.0)
    p.add_argument("--overlap-s", type=float, default=6.0)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--device", default="auto", choices=["auto", "cpu", "mps", "cuda"])
    return p.parse_args()


def load_wav_16k_mono(path: Path):
    import soundfile as sf

    samples, rate = sf.read(str(path), dtype="float32", always_2d=False)
    if rate != SAMPLE_RATE:
        sys.exit(
            f"expected {SAMPLE_RATE}Hz, got {rate}Hz -- resample first, e.g.: "
            f"ffmpeg -i {path} -ac 1 -ar {SAMPLE_RATE} out.wav"
        )
    if samples.ndim != 1:
        sys.exit(
            f"expected mono, got {samples.shape[1]} channels -- downmix first, e.g.: "
            f"ffmpeg -i {path} -ac 1 -ar {SAMPLE_RATE} out.wav"
        )
    return samples


def pick_device(requested: str) -> str:
    import torch

    if requested != "auto":
        return requested
    if torch.cuda.is_available():
        return "cuda"
    if torch.backends.mps.is_available():
        return "mps"
    return "cpu"


def extract_segments(hyp, chunk_start_s: float) -> tuple[list[dict], bool]:
    """Pull segment-level (start, end, text) out of a NeMo Hypothesis, offset
    into the original recording's timeline. Returns (segments, exact) --
    exact=False means no usable per-segment timing came back and the whole
    chunk was returned as one un-timed block instead (see module docstring).
    """
    ts = getattr(hyp, "timestamp", None)
    if isinstance(ts, dict):
        seg_list = ts.get("segment") or ts.get("segments")
        if seg_list:
            out = []
            for s in seg_list:
                start = s.get("start") or s.get("start_offset")
                end = s.get("end") or s.get("end_offset")
                text = s.get("segment") or s.get("text") or s.get("string")
                if start is None or end is None or text is None:
                    continue
                out.append({"start": float(start) + chunk_start_s, "end": float(end) + chunk_start_s, "text": text})
            if out:
                return out, True
    # Fallback: no usable timestamps -- one un-timed segment spanning the
    # whole chunk. Caller fills in the real end time.
    return [{"start": chunk_start_s, "end": None, "text": hyp.text}], False


def main() -> None:
    args = parse_args()
    model_name = args.model or DEFAULT_MODELS[args.engine]

    if args.overlap_s >= args.window_s:
        sys.exit(f"--overlap-s ({args.overlap_s}) must be smaller than --window-s ({args.window_s})")

    samples = load_wav_16k_mono(args.input)
    total_samples = len(samples)
    total_s = total_samples / SAMPLE_RATE
    print(f"Loaded {total_samples} samples ({total_s:.1f}s) from {args.input}", file=sys.stderr)

    import torch
    from nemo.collections.asr.models import ASRModel

    device = pick_device(args.device)
    torch.manual_seed(0)  # decoding here is greedy/deterministic; belt-and-braces only.
    print(f"Loading {model_name} on {device}...", file=sys.stderr)
    model = ASRModel.from_pretrained(model_name, map_location=device)
    model.eval()

    window_samples = int(args.window_s * SAMPLE_RATE)
    overlap_samples = int(args.overlap_s * SAMPLE_RATE)
    hop_samples = window_samples - overlap_samples

    chunks = []
    all_exact = True
    start = 0
    index = 0
    with tempfile.TemporaryDirectory() as tmpdir:
        import soundfile as sf

        while True:
            end = min(start + window_samples, total_samples)
            chunk_audio = samples[start:end]
            chunk_start_s = start / SAMPLE_RATE
            chunk_end_s = end / SAMPLE_RATE
            print(f"Chunk {index}: [{chunk_start_s:.1f}s, {chunk_end_s:.1f}s] ({len(chunk_audio)} samples)...", file=sys.stderr)

            tmp_wav = Path(tmpdir) / f"chunk_{index}.wav"
            sf.write(str(tmp_wav), chunk_audio, SAMPLE_RATE, subtype="PCM_16")

            call_start = time.monotonic()
            try:
                hyps = model.transcribe([str(tmp_wav)], timestamps=True, batch_size=1)
                segments, exact = extract_segments(hyps[0], chunk_start_s)
            except Exception as e:  # noqa: BLE001 -- e.g. the known Canary+timestamps NeMo bug
                print(f"  timestamps=True failed ({e!r}); retrying without timestamps", file=sys.stderr)
                hyps = model.transcribe([str(tmp_wav)], timestamps=False, batch_size=1)
                text = hyps[0].text if hasattr(hyps[0], "text") else str(hyps[0])
                segments, exact = [{"start": chunk_start_s, "end": None, "text": text}], False
            transcribe_seconds = time.monotonic() - call_start

            for s in segments:
                if s["end"] is None:
                    s["end"] = chunk_end_s
            all_exact = all_exact and exact

            chunks.append(
                {
                    "index": index,
                    "start": chunk_start_s,
                    "end": chunk_end_s,
                    "transcribe_seconds": transcribe_seconds,
                    "segments": segments,
                }
            )

            if end >= total_samples:
                break
            start += hop_samples
            index += 1

    run = {
        "window_s": args.window_s,
        "overlap_s": args.overlap_s,
        "input": str(args.input),
        "engine": args.engine,
        "model": model_name,
        "segment_timestamps": "exact" if all_exact else "estimated",
        "chunks": chunks,
    }
    args.out.write_text(json.dumps(run, indent=2))
    print(f"Wrote {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
