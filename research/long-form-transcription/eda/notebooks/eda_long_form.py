import marimo

__generated_with = "0.9.14"
app = marimo.App(width="medium")


@app.cell
def _():
    import json
    import sys
    from pathlib import Path

    import marimo as mo

    EDA_DIR = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(EDA_DIR))

    from chunk_merge import Chunk, merge_chunks
    from wer import word_error_rate

    return Chunk, EDA_DIR, Path, json, merge_chunks, mo, word_error_rate


@app.cell
def _(mo):
    mo.md(
        """
        # Long-form transcription: rolling-window EDA

        Loads chunked-transcription runs (real `chunk_harness` output, or the
        bundled synthetic fixtures) and runs them through the merge algorithm
        in `chunk_merge.py`, so we can compare window/overlap settings on
        boundary quality and, when a reference transcript exists, WER.

        **This notebook alone does not tell you whether Parakeet handles long
        recordings well** -- the synthetic fixtures are a mechanically
        generated stand-in with no model in the loop, useful only for
        validating the merge/metrics pipeline itself. Point `run_dir` below
        at real `chunk_harness` output (see ../../README.md) to get a real
        answer.
        """
    )
    return


@app.cell
def _(EDA_DIR, mo):
    run_dir_picker = mo.ui.text(
        value=str(EDA_DIR / "fixtures"),
        label="Directory of chunk-run JSON files (chunk_harness --out, or fixtures/)",
        full_width=True,
    )
    run_dir_picker
    return (run_dir_picker,)


@app.cell
def _(Path, json, run_dir_picker):
    run_dir = Path(run_dir_picker.value)
    run_files = sorted(run_dir.glob("*.json")) if run_dir.exists() else []
    runs = {}
    for f in run_files:
        try:
            runs[f.stem] = json.loads(f.read_text())
        except (json.JSONDecodeError, OSError):
            continue  # skip non-run JSON dropped in the same directory
    reference_path = run_dir / "reference.txt"
    reference_text = reference_path.read_text().strip() if reference_path.exists() else None
    runs, reference_text
    return reference_text, run_dir, run_files, runs


@app.cell
def _(mo, run_files, runs):
    mo.md(
        f"Found **{len(runs)}** run(s) in the directory ({len(run_files)} JSON files scanned): "
        + ", ".join(sorted(runs)) if runs else "No run JSON files found in that directory."
    )
    return


@app.cell
def _(Chunk, merge_chunks, reference_text, runs, word_error_rate):
    # Run every (window, overlap) config through the merge algorithm and
    # collect the metrics that matter for judging chunking quality:
    #  - boundary_count / low_confidence_count: how often chunks disagreed
    #    enough at the seam to fall back to a lossy timestamp cut
    #  - wer: only meaningful when reference.txt sits next to the run files
    #  - words_per_audio_second: a cheap hallucination/drop smell test --
    #    should track the run's own words-per-second rate; a sharp spike or
    #    drop suggests the model is repeating itself or losing content over
    #    a long chunk, independent of the merge step
    results = {}
    for name, data in runs.items():
        chunks = [Chunk.from_dict(c) for c in data.get("chunks", [])]
        if not chunks:
            continue
        merged = merge_chunks(chunks)
        duration = max(c.end for c in chunks) - min(c.start for c in chunks)
        low_conf = sum(1 for b in merged.boundaries if b.low_confidence)
        results[name] = {
            "window_s": data.get("window_s"),
            "overlap_s": data.get("overlap_s"),
            "n_chunks": len(chunks),
            "boundary_count": len(merged.boundaries),
            "low_confidence_count": low_conf,
            "low_confidence_rate": (low_conf / len(merged.boundaries)) if merged.boundaries else 0.0,
            "words_per_audio_second": len(merged.words) / duration if duration else 0.0,
            "wer": word_error_rate(reference_text, merged.text) if reference_text else None,
            "text": merged.text,
        }
    results
    return chunks, data, duration, low_conf, merged, name, results


@app.cell
def _(mo, results):
    mo.ui.table(
        [
            {k: v for k, v in row.items() if k != "text"} | {"run": run_name}
            for run_name, row in results.items()
        ],
        label="Per-run metrics (sort by column header)",
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## Reading this table

        - **low_confidence_rate = 0** across the board with real audio would
          be the strong signal that overlap is large enough and the merge
          heuristic is finding it reliably -- worth checking whether that
          holds as window size grows toward the 2-3 hour range this feature
          needs, not just on a 5-minute test clip.
        - **wer** (only populated when a hand-written `reference.txt` sits
          next to the run) is the number that actually answers "does
          chunking preserve quality" -- everything else here is a proxy.
        - A believable failure mode to watch for once real audio is in the
          loop: WER *rising* with window size, which would say Parakeet
          itself degrades on longer single passes (the user's original
          hypothesis) rather than the merge step being at fault -- compare
          against a same-window/zero-overlap run (no merge involved) to
          isolate the two.
        """
    )
    return


@app.cell
def _(mo, results):
    run_select = mo.ui.dropdown(sorted(results), value=sorted(results)[0] if results else None, label="Inspect merged text for run")
    run_select
    return (run_select,)


@app.cell
def _(mo, results, run_select):
    mo.md(f"```\n{results[run_select.value]['text']}\n```") if run_select.value else mo.md("No run selected.")
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## Next steps to get a real answer

        1. Record (or find) a 2-3 hour sample with a hand-checked reference
           transcript for at least a 5-10 minute slice -- WER on synthetic
           fixtures proves nothing about real model behavior.
        2. Build the model: `cargo build --release -p chunk_harness` in
           `../chunk_harness`.
        3. Run it at a few window/overlap settings, e.g.:
           `chunk_harness --model-dir <parakeet dir> --input meeting.wav
           --window-s 30 --overlap-s 6 --out fixtures/real_w30_o6.json`
        4. Point `run_dir` above at the directory holding those `--out`
           files (plus a `reference.txt`) and re-run this notebook.
        """
    )
    return


if __name__ == "__main__":
    app.run()
