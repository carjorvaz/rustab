#!/usr/bin/env python3

import argparse
import shutil
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
EXTENSIONS_DIR = REPO_ROOT / "extensions"
STAGED_CONTENTS = {
    "required": (
        ("browser", "manifest.json", "manifest.json"),
        ("browser", "background.js", "background.js"),
        ("browser", "icon48.png", "icon48.png"),
        ("browser", "icon128.png", "icon128.png"),
        ("shared", "background_core.js", "background_core.js"),
    ),
    "optional": {
        "firefox": (("browser", ".amo-upload-uuid", ".amo-upload-uuid"),),
    },
}
SUPPORTED_BROWSERS = {"chrome", "firefox", "orion"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage a browser extension with the shared background core."
    )
    parser.add_argument("browser", choices=sorted(SUPPORTED_BROWSERS))
    parser.add_argument("out_dir", type=Path)
    return parser.parse_args()


def ensure_safe_output_dir(out_dir: Path) -> None:
    if out_dir == REPO_ROOT:
        raise ValueError(f"refusing to replace repository root: {out_dir}")

    try:
        out_dir.relative_to(EXTENSIONS_DIR)
    except ValueError:
        return

    raise ValueError(f"refusing to replace extension source path: {out_dir}")


def reset_output_dir(out_dir: Path) -> None:
    if out_dir.exists():
        if out_dir.is_dir():
            shutil.rmtree(out_dir)
        else:
            out_dir.unlink()
    out_dir.mkdir(parents=True)


def stage_extension(browser: str, out_dir: Path) -> Path:
    if browser not in SUPPORTED_BROWSERS:
        raise ValueError(
            f"unsupported browser {browser!r}; expected one of {sorted(SUPPORTED_BROWSERS)}"
        )

    browser_dir = EXTENSIONS_DIR / browser
    ensure_safe_output_dir(out_dir)
    reset_output_dir(out_dir)

    source_roots = {
        "browser": browser_dir,
        "shared": EXTENSIONS_DIR / "shared",
    }
    for source_root, source_name, destination_name in STAGED_CONTENTS["required"]:
        shutil.copy2(
            source_roots[source_root] / source_name, out_dir / destination_name
        )

    for source_root, source_name, destination_name in STAGED_CONTENTS["optional"].get(
        browser, ()
    ):
        source_path = source_roots[source_root] / source_name
        if source_path.exists():
            shutil.copy2(source_path, out_dir / destination_name)

    return out_dir


def main() -> int:
    args = parse_args()
    stage_extension(args.browser, args.out_dir.expanduser().resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
