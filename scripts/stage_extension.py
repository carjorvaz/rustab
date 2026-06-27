#!/usr/bin/env python3

import argparse
import shutil
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
EXTENSIONS_DIR = REPO_ROOT / "extensions"
STAGED_EXTENSION_FILES = [
    "manifest.json",
    "background.js",
    "icon48.png",
    "icon128.png",
]
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

    for relative_name in STAGED_EXTENSION_FILES:
        shutil.copy2(browser_dir / relative_name, out_dir / relative_name)
    shutil.copy2(
        EXTENSIONS_DIR / "shared" / "background_core.js", out_dir / "background_core.js"
    )

    amo_upload_uuid = browser_dir / ".amo-upload-uuid"
    if browser == "firefox" and amo_upload_uuid.exists():
        shutil.copy2(amo_upload_uuid, out_dir / ".amo-upload-uuid")

    return out_dir


def main() -> int:
    args = parse_args()
    stage_extension(args.browser, args.out_dir.expanduser().resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
