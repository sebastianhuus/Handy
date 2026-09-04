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

    Loads **pre-computed** metrics (`evaluate.py --out-dir`'s
    `*.summary.json` files) -- this notebook never re-runs the merge
    algorithm or WER scoring itself, so opening it doesn't cost anyone
    the compute of regenerating a sweep just to look at existing
    results. Points at `fixtures_summaries/` by default (synthetic,
    bundled, no model needed); point it at your own summaries directory
    (e.g. `../real_runs/summaries/`, gitignored -- real recordings and
    their transcripts stay local) to browse a real sweep instead.

    Use the **"Regenerate summaries"** section below only when you've
    produced new run JSON (via `../chunk_harness` or
    `../model_harness`) and actually want to re-score it -- that's the
    one thing in this notebook that costs real compute (though
    `evaluate.py` itself is fast; the model runs that produce its
    input are not).
    """)
    return


@app.cell
def _(EDA_DIR, mo):
    summaries_dir_picker = mo.ui.text(
        value=str(EDA_DIR / "fixtures_summaries"),
        label="Directory of *.summary.json files to browse",
        full_width=True,
    )
    summaries_dir_picker
    return (summaries_dir_picker,)


@app.cell
def _(mo):
    mo.md("""
    ## Regenerate summaries (optional -- runs `evaluate.py` for you)
    """)
    return


@app.cell
def _(EDA_DIR, mo, summaries_dir_picker):
    regen_run_dir = mo.ui.text(value=str(EDA_DIR / "fixtures"), label="--run-dir (directory of run JSON from either harness)", full_width=True)
    regen_reference = mo.ui.text(value=str(EDA_DIR / "fixtures" / "reference.txt"), label="--reference (leave blank to skip WER)", full_width=True)
    regen_out_dir = mo.ui.text(value=summaries_dir_picker.value, label="--out-dir (defaults to the directory being browsed above)", full_width=True)
    regen_strip_fillers = mo.ui.checkbox(value=True, label="--strip-fillers")
    regen_word_limit = mo.ui.text(value="", label="--wer-word-limit (blank = full document)")
    regen_button = mo.ui.run_button(label="Run evaluate.py")
    mo.vstack([regen_run_dir, regen_reference, regen_out_dir, regen_strip_fillers, regen_word_limit, regen_button])
    return (
        regen_button,
        regen_out_dir,
        regen_reference,
        regen_run_dir,
        regen_strip_fillers,
        regen_word_limit,
    )


@app.cell
def _(
    EDA_DIR,
    mo,
    regen_button,
    regen_out_dir,
    regen_reference,
    regen_run_dir,
    regen_strip_fillers,
    regen_word_limit,
    subprocess,
    sys,
):
    # Deliberately not `mo.stop`-gated: this cell must produce
    # `regen_run_count` on *every* run, including the very first (unclicked)
    # one, since the loader cell below depends on it to know when to re-read
    # the summaries directory. `mo.stop` raises before a `return` executes,
    # which would make that dependency never resolve until first clicked --
    # a real bug caught by `marimo export html`/`export script`, not by eye.
    if regen_button.value:
        _cmd = [
            sys.executable,
            "evaluate.py",
            "--run-dir",
            regen_run_dir.value,
            "--out-dir",
            regen_out_dir.value,
        ]
        if regen_reference.value.strip():
            _cmd += ["--reference", regen_reference.value]
        if regen_strip_fillers.value:
            _cmd += ["--strip-fillers"]
        if regen_word_limit.value.strip():
            _cmd += ["--wer-word-limit", regen_word_limit.value.strip()]

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
    regen_run_count = 1 if regen_button.value else 0
    regen_output
    return (regen_run_count,)


@app.cell
def _(Path, json, mo, regen_run_count, summaries_dir_picker):
    del regen_run_count  # see the comment on the cell that produces it

    _dir = Path(summaries_dir_picker.value)
    summary_files = sorted(_dir.glob("*.summary.json")) if _dir.exists() else []
    summaries = []
    for _f in summary_files:
        try:
            summaries.append(json.loads(_f.read_text()))
        except (json.JSONDecodeError, OSError):
            continue

    mo.md(
        f"Found **{len(summaries)}** summary file(s) in `{_dir}`."
        if summaries
        else f"No `*.summary.json` files found in `{_dir}` -- run `evaluate.py --out-dir` "
        "against a run directory first (see the regenerate section above, or the README)."
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
        wer_plot = mo.md("*(no `--reference` was given for any of these runs -- no WER to plot)*")
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
    mo.stop(not _rows, mo.md("*(no run here has timing info to plot realtime factor)*"))

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

    - **WER**: the number that actually answers "does this config
      produce a usable transcript" -- everything else here is a proxy.
      Only populated when the summaries were generated with
      `--reference`.
    - **low_confidence_rate**: fraction of chunk-to-chunk merge
      boundaries that fell back to a lossy timestamp cut instead of a
      confident text match. High values mean the overlap window isn't
      reliably catching real speech overlap -- see SPEC.md §4.
    - **realtime factor**: how many seconds of audio get transcribed
      per second of compute. Only meaningful for comparing *engines* on
      the *same machine* -- don't compare across machines with this.
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
def _(Path, mo, run_select, summaries_dir_picker):
    mo.stop(run_select.value is None)
    _dir = Path(summaries_dir_picker.value)
    _raw_path = _dir / f"{run_select.value}.merged.txt"
    _cleaned_path = _dir / f"{run_select.value}.merged.cleaned.txt"
    _raw_text = _raw_path.read_text() if _raw_path.exists() else "(not found -- was this generated with evaluate.py --out-dir?)"
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

    1. Produce run JSON with `../chunk_harness` (Rust, real Parakeet)
       or `../model_harness` (Python, NeMo -- Parakeet or Canary).
    2. Score it: `uv run python3 evaluate.py --run-dir <dir> --reference
       <ref> --strip-fillers --out-dir <summaries_dir>` -- or use the
       "Regenerate summaries" section above to do the same thing from
       here.
    3. Point the picker at the top of this notebook at that
       `<summaries_dir>` to browse the results.

    See `../README.md` for the full commands and `../SPEC.md` /
    `../eda/real_runs/FINDINGS.md` for what this research has found so
    far.
    """)
    return


if __name__ == "__main__":
    app.run()
