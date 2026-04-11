#!/usr/bin/env python3

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional
import xml.etree.ElementTree as ET

REPO_ROOT = Path(__file__).resolve().parent.parent
EXTENSION_DIR = REPO_ROOT / "extensions" / "chrome"
DEFAULT_EXTENSION_ID = "nddbmnpippfilnjoebpcnfbpebnllbgo"
EXTENSION_FILES = [
    "manifest.json",
    "background.js",
    "icon48.png",
    "icon128.png",
]

MAC_BROWSER_CANDIDATES = [
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
]

LINUX_BROWSER_CANDIDATES = [
    "brave-browser",
    "google-chrome-stable",
    "google-chrome",
    "chromium",
    "chromium-browser",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Package rustab's Chromium extension into a managed-distribution "
            "bundle containing a .crx, updates.xml, and an ExtensionSettings snippet."
        )
    )
    parser.add_argument(
        "--key",
        required=True,
        type=Path,
        help=(
            "Path to the Chromium extension private key (.pem) matching the "
            "public key embedded in extensions/chrome/manifest.json."
        ),
    )
    parser.add_argument(
        "--base-url",
        required=True,
        help=(
            "Public base URL that will host updates.xml and "
            "extension-settings.json."
        ),
    )
    parser.add_argument(
        "--codebase-url",
        default=None,
        help=(
            "Exact public URL for the packaged .crx file. Defaults to "
            "<base-url>/rustab-<version>.crx."
        ),
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=REPO_ROOT / "dist" / "chromium",
        help="Directory where packaged artifacts should be written.",
    )
    parser.add_argument(
        "--browser-binary",
        type=Path,
        default=None,
        help="Path to a Chromium-family browser binary that supports --pack-extension.",
    )
    parser.add_argument(
        "--extension-id",
        default=DEFAULT_EXTENSION_ID,
        help="Chromium extension ID. Defaults to rustab's stable ID.",
    )
    parser.add_argument(
        "--installation-mode",
        default="force_installed",
        choices=["force_installed", "normal_installed"],
        help="Managed installation mode to place in the generated policy snippet.",
    )
    return parser.parse_args()


def find_browser_binary(explicit: Optional[Path]) -> Path:
    if explicit is not None:
        if explicit.exists():
            return explicit
        raise FileNotFoundError(f"browser binary not found: {explicit}")

    candidates = MAC_BROWSER_CANDIDATES if sys.platform == "darwin" else LINUX_BROWSER_CANDIDATES
    for candidate in candidates:
        path = Path(candidate)
        if path.is_absolute():
            if path.exists():
                return path
        else:
            resolved = shutil.which(candidate)
            if resolved:
                return Path(resolved)

    raise FileNotFoundError(
        "could not find a Chromium-family browser binary; pass --browser-binary explicitly"
    )


def read_manifest(path: Path) -> dict:
    return json.loads(path.read_text())


def write_manifest(path: Path, manifest: dict) -> None:
    path.write_text(json.dumps(manifest, indent=2) + "\n")


def render_updates_xml(extension_id: str, codebase_url: str, version: str) -> str:
    root = ET.Element(
        "gupdate",
        attrib={
            "xmlns": "http://www.google.com/update2/response",
            "protocol": "2.0",
        },
    )
    app = ET.SubElement(root, "app", attrib={"appid": extension_id})
    ET.SubElement(
        app,
        "updatecheck",
        attrib={
            "codebase": codebase_url,
            "version": version,
        },
    )
    return "<?xml version='1.0' encoding='UTF-8'?>\n" + ET.tostring(
        root, encoding="unicode"
    )


def render_extension_settings(update_url: str, installation_mode: str) -> dict:
    return {
        "installation_mode": installation_mode,
        "update_url": update_url,
        # Keep future updates pinned to the same hosted update URL instead of
        # requiring the extension manifest itself to carry production policy.
        "override_update_url": True,
    }


def package_extension(
    browser_binary: Path,
    extension_dir: Path,
    key_path: Path,
) -> Path:
    command = [
        str(browser_binary),
        f"--pack-extension={extension_dir}",
        f"--pack-extension-key={key_path}",
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(
            "browser packaging command failed:\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )

    crx_path = extension_dir.with_suffix(".crx")
    if not crx_path.exists():
        raise FileNotFoundError(f"expected packaged extension at {crx_path}")
    return crx_path


def main() -> int:
    args = parse_args()
    key_path = args.key.expanduser().resolve()
    if not key_path.exists():
        print(f"key not found: {key_path}", file=sys.stderr)
        return 1

    browser_binary = find_browser_binary(args.browser_binary)
    base_url = args.base_url.rstrip("/")
    out_dir = args.out_dir.expanduser().resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    manifest = read_manifest(EXTENSION_DIR / "manifest.json")
    version = manifest["version"]
    crx_filename = f"rustab-{version}.crx"
    update_url = f"{base_url}/updates.xml"
    codebase_url = args.codebase_url or f"{base_url}/{crx_filename}"

    with tempfile.TemporaryDirectory(prefix="rustab-chromium-release-") as temp_dir_name:
        temp_dir = Path(temp_dir_name)
        staged_extension_dir = temp_dir / "rustab"
        staged_extension_dir.mkdir()
        for relative_name in EXTENSION_FILES:
            shutil.copy2(EXTENSION_DIR / relative_name, staged_extension_dir / relative_name)

        staged_manifest_path = staged_extension_dir / "manifest.json"
        staged_manifest_path.chmod(0o644)
        staged_manifest = read_manifest(staged_manifest_path)
        staged_manifest["update_url"] = update_url
        write_manifest(staged_manifest_path, staged_manifest)

        crx_path = package_extension(browser_binary, staged_extension_dir, key_path)
        final_crx_path = out_dir / crx_filename
        shutil.move(crx_path, final_crx_path)

    updates_xml_path = out_dir / "updates.xml"
    updates_xml_path.write_text(
        render_updates_xml(args.extension_id, codebase_url, version) + "\n"
    )

    extension_settings = {
        args.extension_id: render_extension_settings(update_url, args.installation_mode)
    }
    (out_dir / "extension-settings.json").write_text(
        json.dumps({"ExtensionSettings": extension_settings}, indent=2) + "\n"
    )

    print(f"browser binary: {browser_binary}")
    print(f"packaged CRX: {final_crx_path}")
    print(f"update manifest: {updates_xml_path}")
    print(f"policy snippet: {out_dir / 'extension-settings.json'}")
    print(f"extension id: {args.extension_id}")
    print(f"hosted update URL: {update_url}")
    print(f"CRX download URL: {codebase_url}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
