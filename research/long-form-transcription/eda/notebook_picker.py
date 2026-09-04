"""`uv run notebook` -> launches this project's one marimo notebook with --no-token --watch."""

import subprocess
from pathlib import Path

ROOT = Path(__file__).parent
NOTEBOOK = ROOT / "notebooks" / "eda_long_form.py"


def main() -> None:
    print(f"\nLaunching: uv run marimo edit --no-token --watch {NOTEBOOK.relative_to(ROOT)}\n")
    subprocess.run(
        ["uv", "run", "marimo", "edit", "--no-token", "--watch", str(NOTEBOOK)],
        cwd=ROOT,
        check=True,
    )


if __name__ == "__main__":
    main()
