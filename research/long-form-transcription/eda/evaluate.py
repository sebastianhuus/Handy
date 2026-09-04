#!/usr/bin/env python3
"""Reproducible metrics CLI for chunk-run JSON files -- the checked-in,
documented version of the ad-hoc analysis used throughout this research
(see ../SPEC.md and real_runs/FINDINGS.md). Deterministic and dependency-
free (stdlib only, same as chunk_merge.py/postprocess.py/wer.py): given the
same run JSON and reference text, always produces the same numbers, on any
machine.

Works on run JSON from *either* harness -- ../chunk_harness (Rust,
transcribe-rs/ONNX, what the app would actually ship) or
../model_harness (Python, NeMo reference implementations, for evaluating
candidate engines before deciding what -- if anything -- changes in the
app). Both emit the same schema (see either harness's own docs), so this
script doesn't care which one produced a given file.

Usage:
    python3 evaluate.py --run-dir some_dir/ [--reference reference.txt]
    python3 evaluate.py --run a.json --run b.json [--reference ref.txt]

With --reference: computes real WER (raw merge, and after the
repeat-collapse post-processing filter) against a hand-checked or
otherwise-trusted transcript. Without it: proxy metrics only (matches
SPEC.md §6's list of what's available before a real reference exists).

--out-dir, if given, writes each run's merged text (raw and
repeat-collapsed) next to a JSON summary -- for spot-checking or feeding
into a diff, same as the *.merged.txt files used throughout this research.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from chunk_merge import Chunk, merge_chunks  # noqa: E402
from postprocess import collapse_repeated_words  # noqa: E402
from wer import word_error_rate  # noqa: E402


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--run", action="append", default=[], type=Path, help="a single run JSON file (repeatable)")
    p.add_argument("--run-dir", type=Path, default=None, help="directory of run JSON files (non-recursive, *.json)")
    p.add_argument("--reference", type=Path, default=None, help="hand-checked (or otherwise trusted) reference transcript, plain text")
    p.add_argument("--out-dir", type=Path, default=None, help="write merged text + a JSON summary per run here")
    p.add_argument("--wer-word-limit", type=int, default=None, help="cap WER computation to the first N reference words (WER is O(n*m); omit for full-document on short references, set this for long ones)")
    p.add_argument(
        "--strip-fillers",
        action="store_true",
        help=(
            "drop bare filler tokens (uh/um/erm/ah) from both the reference and every "
            "hypothesis before scoring. Off by default -- this is a real normalization "
            "choice, not a neutral default, so it's opt-in rather than silent. Matters a "
            "lot in practice: different engines/references keep wildly different amounts "
            "of filler words by convention (not by transcription accuracy), which can "
            "swamp real content-word errors in the raw WER number -- see "
            "real_runs/FINDINGS.md's before/after numbers for how much this moves things."
        ),
    )
    return p.parse_args()


def collect_run_files(args: argparse.Namespace) -> list[Path]:
    files = list(args.run)
    if args.run_dir:
        files += sorted(args.run_dir.glob("*.json"))
    if not files:
        sys.exit("no run files given -- pass --run and/or --run-dir")
    return files


FILLERS = {"uh", "um", "uhh", "umm", "erm", "ah"}


def evaluate_run(
    path: Path,
    reference_words: list[str] | None,
    wer_word_limit: int | None,
    strip_fillers: bool = False,
) -> dict:
    data = json.loads(path.read_text())
    chunks = [Chunk.from_dict(c) for c in data.get("chunks", [])]
    result = {
        "run": path.stem,
        "engine": data.get("engine", "unknown"),
        "model": data.get("model"),
        "window_s": data.get("window_s"),
        "overlap_s": data.get("overlap_s"),
        "segment_timestamps": data.get("segment_timestamps", "exact"),  # chunk_harness (Rust) predates this field; exact is its actual behavior
    }
    if not chunks:
        result["error"] = "no chunks in run file"
        return result

    merged = merge_chunks(chunks)
    duration = max(c.end for c in chunks) - min(c.start for c in chunks)
    low_conf = sum(1 for b in merged.boundaries if b.low_confidence)
    total_transcribe_s = sum(c.get("transcribe_seconds", 0.0) for c in data.get("chunks", []))

    cleaned_words = collapse_repeated_words(merged.words)

    result.update(
        {
            "n_chunks": len(chunks),
            "boundaries": len(merged.boundaries),
            "low_confidence_count": low_conf,
            "low_confidence_rate": round(low_conf / len(merged.boundaries), 4) if merged.boundaries else 0.0,
            "words_raw": len(merged.words),
            "words_after_repeat_collapse": len(cleaned_words),
            "words_per_audio_second": round(len(merged.words) / duration, 3) if duration else 0.0,
            "total_transcribe_seconds": round(total_transcribe_s, 2),
            "realtime_factor": round(duration / total_transcribe_s, 1) if total_transcribe_s else None,
        }
    )

    if reference_words is not None:
        ref = reference_words
        raw_hyp, cleaned_hyp = merged.words, cleaned_words
        if strip_fillers:
            ref = [w for w in ref if w not in FILLERS]
            raw_hyp = [w for w in raw_hyp if w not in FILLERS]
            cleaned_hyp = [w for w in cleaned_hyp if w not in FILLERS]
        if wer_word_limit:
            ref, raw_hyp, cleaned_hyp = ref[:wer_word_limit], raw_hyp[:wer_word_limit], cleaned_hyp[:wer_word_limit]
        result["wer_raw"] = round(word_error_rate(" ".join(ref), " ".join(raw_hyp)), 4)
        result["wer_after_repeat_collapse"] = round(word_error_rate(" ".join(ref), " ".join(cleaned_hyp)), 4)

    result["_merged_text_raw"] = merged.text
    result["_merged_text_cleaned"] = " ".join(cleaned_words)
    return result


def normalize_reference(text: str) -> list[str]:
    """Same normalization the rest of this research applies before WER: this
    is a policy choice, not a neutral default -- see SPEC.md §6 / wer.py's
    own docstring on why WER is case/punctuation-sensitive by design. Callers
    wanting raw-vs-raw WER should pre-normalize both sides identically
    themselves; this CLI normalizes because none of the candidate engines'
    raw casing/punctuation conventions agree with each other or a typical
    hand-written reference, which would otherwise swamp real errors with
    convention noise (see real_runs/FINDINGS.md's "wanna" vs "want to"
    discussion).
    """
    import re

    return re.sub(r"[^\w' ]+", " ", text.lower()).split()


def write_summary(row: dict, out_dir: Path) -> None:
    """Write one evaluate_run() result to out_dir: <run>.summary.json plus
    its raw and repeat-collapsed merged text. Shared by the CLI (main())
    and the notebook's in-process "Regenerate summaries" button -- the
    notebook calls evaluate_run() directly (not via subprocess) so it can
    show live per-run progress, but the output format must stay identical
    either way, hence the shared helper rather than two copies of this.
    """
    if "_merged_text_raw" not in row:
        return
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / f"{row['run']}.merged.txt").write_text(row["_merged_text_raw"])
    (out_dir / f"{row['run']}.merged.cleaned.txt").write_text(row["_merged_text_cleaned"])
    summary = {k: v for k, v in row.items() if not k.startswith("_")}
    (out_dir / f"{row['run']}.summary.json").write_text(json.dumps(summary, indent=2))


def print_table(rows: list[dict]) -> None:
    cols = [c for c in rows[0] if not c.startswith("_")]
    widths = {c: max(len(c), *(len(str(r.get(c, ""))) for r in rows)) for c in cols}
    print(" | ".join(c.ljust(widths[c]) for c in cols))
    print("-+-".join("-" * widths[c] for c in cols))
    for r in rows:
        print(" | ".join(str(r.get(c, "")).ljust(widths[c]) for c in cols))


def main() -> None:
    args = parse_args()
    files = collect_run_files(args)

    reference_words = None
    if args.reference:
        reference_words = normalize_reference(args.reference.read_text())

    rows = [evaluate_run(f, reference_words, args.wer_word_limit, args.strip_fillers) for f in files]

    if args.out_dir:
        for r in rows:
            write_summary(r, args.out_dir)

    print_table(rows)


if __name__ == "__main__":
    main()
