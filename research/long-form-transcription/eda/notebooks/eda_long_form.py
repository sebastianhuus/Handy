import marimo

__generated_with = "0.24.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import json
    import subprocess
    import sys
    from pathlib import Path

    import marimo as mo
    import matplotlib.pyplot as plt

    EDA_DIR = Path(__file__).resolve().parents[1]
    return EDA_DIR, Path, json, mo, plt, subprocess, sys


@app.cell
def _(mo):
    mo.md("""
    # Long-form transcription: rolling-window EDA

    **One choice drives everything below**: pick a dataset directory (a
    folder of run JSON from `../chunk_harness` or `../model_harness`, plus
    an optional `reference.txt`), and the summaries directory, reference
    path, and everything else are derived from it -- no paths to type.
    """)
    return


@app.cell
def _(EDA_DIR):
    # A "dataset" is any top-level directory under eda/ holding run JSON
    # directly (not recursively -- eda/fixtures/summaries/ has JSON too, but
    # that's evaluate.py's *output*, not a dataset to regenerate from).
    _skip = {"notebooks", ".venv", "__pycache__"}
    dataset_dirs = sorted(
        d
        for d in EDA_DIR.iterdir()
        if d.is_dir() and d.name not in _skip and not d.name.startswith(".") and any(d.glob("*.json"))
    )
    return (dataset_dirs,)


@app.cell
def _(EDA_DIR, dataset_dirs, mo):
    mo.stop(
        not dataset_dirs,
        mo.md(
            f"**No dataset directories found under `{EDA_DIR}`** -- a dataset "
            "is any top-level folder containing `*.json` run files, e.g. "
            "`fixtures/` (bundled, synthetic) or `real_runs/` (your own, "
            "gitignored). Produce some with `../chunk_harness` or "
            "`../model_harness` first."
        ),
    )
    dataset_picker = mo.ui.dropdown(
        {d.name: d for d in dataset_dirs},
        value=dataset_dirs[0].name,
        label="Dataset",
    )
    dataset_picker
    return (dataset_picker,)


@app.cell
def _(dataset_picker, mo):
    dataset_dir = dataset_picker.value
    reference_path = dataset_dir / "reference.txt"
    summaries_dir = dataset_dir / "summaries"
    has_reference = reference_path.exists()

    mo.md(
        f"""
    Derived from **{dataset_dir.name}**:

    - run JSON: `{dataset_dir.name}/*.json`
    - reference: `{dataset_dir.name}/reference.txt`{" ✓ found" if has_reference else " -- **not found, WER will be skipped**"}
    - summaries written to: `{dataset_dir.name}/summaries/`
    """
    )
    return dataset_dir, has_reference, reference_path, summaries_dir


@app.cell
def _(mo):
    strip_fillers_checkbox = mo.ui.checkbox(
        value=True,
        label="Strip filler words (uh/um/erm/ah) before scoring -- recommended; see FINDINGS.md for why this matters for a fair comparison",
    )
    strip_fillers_checkbox
    return (strip_fillers_checkbox,)


@app.cell
def _(mo):
    regenerate_button = mo.ui.run_button(label="Regenerate summaries for this dataset", kind="success", full_width=True)
    regenerate_button
    return (regenerate_button,)


@app.cell
def _(
    EDA_DIR,
    dataset_dir,
    has_reference,
    mo,
    reference_path,
    regenerate_button,
    strip_fillers_checkbox,
    subprocess,
    summaries_dir,
    sys,
):
    # Deliberately not `mo.stop`-gated: this cell must produce
    # `regen_run_count` on *every* run, including the very first (unclicked)
    # one, since the loader cell below depends on it to know when to re-read
    # the summaries directory. `mo.stop` raises before a `return` executes,
    # which would leave that dependency unresolved until first clicked.
    if regenerate_button.value:
        _cmd = [
            sys.executable,
            "evaluate.py",
            "--run-dir",
            str(dataset_dir),
            "--out-dir",
            str(summaries_dir),
        ]
        if has_reference:
            _cmd += ["--reference", str(reference_path)]
        if strip_fillers_checkbox.value:
            _cmd += ["--strip-fillers"]

        _result = subprocess.run(_cmd, cwd=EDA_DIR, capture_output=True, text=True)
        regen_output = mo.vstack(
            [
                mo.md(f"`{' '.join(_cmd)}`"),
                mo.md(f"exit code: {_result.returncode}"),
                mo.md("```\n" + (_result.stdout or "(no stdout)") + "\n```"),
                mo.md("```\n" + _result.stderr + "\n```") if _result.stderr else mo.md(""),
            ]
        )
    else:
        regen_output = mo.md("*(not run yet -- click the button above)*")

    # A run counter, not the button's own value: the button's value flips
    # True then auto-resets to False (per mo.ui.run_button's own semantics),
    # so this cell runs twice per click regardless -- the loader cell below
    # just needs *some* value from this cell to depend on, so it re-runs
    # after either pass and picks up whatever evaluate.py wrote to disk.
    regen_run_count = 1 if regenerate_button.value else 0
    regen_output
    return (regen_run_count,)


@app.cell
def _(json, mo, regen_run_count, summaries_dir):
    del regen_run_count  # see the comment on the cell that produces it

    summary_files = sorted(summaries_dir.glob("*.summary.json")) if summaries_dir.exists() else []
    summaries = []
    for _f in summary_files:
        try:
            summaries.append(json.loads(_f.read_text()))
        except (json.JSONDecodeError, OSError):
            continue

    mo.md(
        f"Found **{len(summaries)}** summary file(s) in `{summaries_dir.name}/`."
        if summaries
        else f"No `*.summary.json` files in `{summaries_dir.name}/` yet -- click "
        "**Regenerate summaries for this dataset** above."
    )
    return (summaries,)


@app.cell
def _(mo, summaries):
    mo.stop(not summaries)
    mo.ui.table(summaries, label="Per-run metrics (sort by column header)")
    return


@app.cell
def _(mo, plt, summaries):
    mo.stop(not summaries)

    _names = [s["run"] for s in summaries]
    _has_wer = any(s.get("wer_raw") is not None for s in summaries)

    if _has_wer:
        _raw = [s.get("wer_raw") or 0 for s in summaries]
        _cleaned = [s.get("wer_after_repeat_collapse") or 0 for s in summaries]
        _x = range(len(_names))
        _fig, _ax = plt.subplots(figsize=(max(6, len(_names) * 1.2), 4))
        _w = 0.35
        _ax.bar([i - _w / 2 for i in _x], _raw, _w, label="raw")
        _ax.bar([i + _w / 2 for i in _x], _cleaned, _w, label="after repeat-collapse")
        _ax.set_xticks(list(_x))
        _ax.set_xticklabels(_names, rotation=30, ha="right")
        _ax.set_ylabel("WER")
        _ax.set_title("Word error rate vs. reference")
        _ax.legend()
        _fig.tight_layout()
        wer_plot = _fig
    else:
        wer_plot = mo.md("*(no `reference.txt` for this dataset -- no WER to plot)*")
    wer_plot
    return


@app.cell
def _(mo, plt, summaries):
    mo.stop(not summaries)

    _names = [s["run"] for s in summaries]
    _low_conf = [s.get("low_confidence_rate") or 0 for s in summaries]
    _fig, _ax = plt.subplots(figsize=(max(6, len(_names) * 1.2), 4))
    _ax.bar(_names, _low_conf, color="#c0392b")
    _ax.set_xticks(range(len(_names)))
    _ax.set_xticklabels(_names, rotation=30, ha="right")
    _ax.set_ylabel("low_confidence_rate")
    _ax.set_ylim(0, 1)
    _ax.set_title("Merge-boundary low-confidence rate (lower is better)")
    _fig.tight_layout()
    _fig
    return


@app.cell
def _(mo, plt, summaries):
    mo.stop(not summaries)

    _rows = [s for s in summaries if s.get("realtime_factor") is not None]
    mo.stop(not _rows, mo.md("*(no run here has timing info to plot realtime factor -- synthetic fixtures never ran a real model, so they have none)*"))

    _names = [s["run"] for s in _rows]
    _rtf = [s["realtime_factor"] for s in _rows]
    _fig, _ax = plt.subplots(figsize=(max(6, len(_names) * 1.2), 4))
    _ax.bar(_names, _rtf, color="#2980b9")
    _ax.set_xticks(range(len(_names)))
    _ax.set_xticklabels(_names, rotation=30, ha="right")
    _ax.set_ylabel("realtime factor (x)")
    _ax.set_title("Transcription throughput (higher is faster)")
    _fig.tight_layout()
    _fig
    return


@app.cell
def _(mo):
    mo.md("""
    ## Reading these plots

    - **WER**: the number that actually answers "does this config produce a
      usable transcript" -- everything else here is a proxy. Only populated
      when this dataset has a `reference.txt`.
    - **low_confidence_rate**: fraction of chunk-to-chunk merge boundaries
      that fell back to a lossy timestamp cut instead of a confident text
      match. High values mean the overlap window isn't reliably catching
      real speech overlap -- see SPEC.md §4.
    - **realtime factor**: how many seconds of audio get transcribed per
      second of compute. Only meaningful for comparing *engines* on the
      *same machine* -- don't compare across machines with this. Synthetic
      fixtures never ran a real model, so they have no timing at all (not
      zero -- genuinely absent, and excluded from this plot rather than
      shown as 0x).
    """)
    return


@app.cell
def _(mo, summaries):
    mo.stop(not summaries)
    _run_names = sorted(s["run"] for s in summaries)
    run_select = mo.ui.dropdown(_run_names, value=_run_names[0], label="Inspect merged text for run")
    run_select
    return (run_select,)


@app.cell
def _(mo, run_select, summaries_dir):
    mo.stop(run_select.value is None)
    _raw_path = summaries_dir / f"{run_select.value}.merged.txt"
    _cleaned_path = summaries_dir / f"{run_select.value}.merged.cleaned.txt"
    _raw_text = _raw_path.read_text() if _raw_path.exists() else "(not found -- try regenerating summaries above)"
    _cleaned_text = _cleaned_path.read_text() if _cleaned_path.exists() else _raw_text
    mo.ui.tabs(
        {
            "raw merge": mo.md(f"```\n{_raw_text}\n```"),
            "after repeat-collapse": mo.md(f"```\n{_cleaned_text}\n```"),
        }
    )
    return


@app.cell
def _(mo):
    mo.md("""
    ## How this fits together

    1. Produce run JSON with `../chunk_harness` (Rust, real Parakeet) or
       `../model_harness` (Python, NeMo -- Parakeet or Canary), writing it
       into its own dataset directory under `eda/` (a new one, or an
       existing one like `real_runs/`).
    2. Pick that directory from the dropdown at the top of this notebook.
    3. Click **Regenerate summaries for this dataset**.

    That's it -- no paths to type. See `../README.md` for the underlying
    commands and `../SPEC.md` / `../eda/real_runs/FINDINGS.md` for what
    this research has found so far.
    """)
    return


if __name__ == "__main__":
    app.run()
