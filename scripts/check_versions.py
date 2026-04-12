#!/usr/bin/env python3

import argparse
import json
import re
import sys
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = REPO_ROOT / "Cargo.toml"
CHROME_MANIFEST = REPO_ROOT / "extensions" / "chrome" / "manifest.json"
FIREFOX_MANIFEST = REPO_ROOT / "extensions" / "firefox" / "manifest.json"
SIGNED_FIREFOX_XPI = REPO_ROOT / "extensions" / "firefox-signed" / "rustab@rustab.dev.xpi"
FIREFOX_EXTENSION_FILES = [
    "manifest.json",
    "background.js",
    "icon48.png",
    "icon128.png",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Verify that Rustab's release version and Firefox extension metadata "
            "stay in sync across Cargo, browser manifests, and the committed "
            "signed XPI."
        )
    )
    parser.add_argument(
        "--print-version",
        action="store_true",
        help="Print the canonical Rustab version after validation succeeds.",
    )
    return parser.parse_args()


def read_json(path: Path) -> dict:
    return json.loads(path.read_text())


def read_workspace_version(path: Path) -> str:
    in_workspace_package = False
    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_workspace_package = line == "[workspace.package]"
            continue
        if not in_workspace_package:
            continue
        match = re.fullmatch(r'version\s*=\s*"([^"]+)"', line)
        if match:
            return match.group(1)
    raise ValueError(f"could not find [workspace.package].version in {path}")


def read_signed_firefox_manifest(path: Path) -> dict:
    with zipfile.ZipFile(path) as archive:
        return json.loads(archive.read("manifest.json"))


def signed_firefox_file_bytes(path: Path, relative_name: str) -> bytes:
    with zipfile.ZipFile(path) as archive:
        return archive.read(relative_name)


def firefox_addon_id(manifest: dict) -> str:
    return manifest["browser_specific_settings"]["gecko"]["id"]


def main() -> int:
    args = parse_args()

    cargo_version = read_workspace_version(CARGO_TOML)
    chrome_manifest = read_json(CHROME_MANIFEST)
    firefox_manifest = read_json(FIREFOX_MANIFEST)
    signed_firefox_manifest = read_signed_firefox_manifest(SIGNED_FIREFOX_XPI)

    observed_versions = {
        "Cargo workspace": cargo_version,
        "Chromium manifest": chrome_manifest["version"],
        "Firefox manifest": firefox_manifest["version"],
        "Signed Firefox XPI": signed_firefox_manifest["version"],
    }

    mismatches = [
        f"{label} has version {observed!r}, expected {cargo_version!r}"
        for label, observed in observed_versions.items()
        if observed != cargo_version
    ]

    firefox_manifest_id = firefox_addon_id(firefox_manifest)
    signed_firefox_id = firefox_addon_id(signed_firefox_manifest)
    if signed_firefox_id != firefox_manifest_id:
        mismatches.append(
            "Signed Firefox XPI has addon id "
            f"{signed_firefox_id!r}, expected {firefox_manifest_id!r}"
        )

    for relative_name in FIREFOX_EXTENSION_FILES:
        source_path = FIREFOX_MANIFEST.parent / relative_name
        if relative_name == "manifest.json":
            source_json = json.loads(source_path.read_text())
            signed_json = json.loads(signed_firefox_file_bytes(SIGNED_FIREFOX_XPI, relative_name))
            if signed_json != source_json:
                mismatches.append(
                    f"Signed Firefox XPI {relative_name} differs from extensions/firefox/{relative_name}"
                )
            continue

        source_bytes = source_path.read_bytes()
        signed_bytes = signed_firefox_file_bytes(SIGNED_FIREFOX_XPI, relative_name)
        if signed_bytes != source_bytes:
            mismatches.append(
                f"Signed Firefox XPI {relative_name} differs from extensions/firefox/{relative_name}"
            )

    if mismatches:
        for mismatch in mismatches:
            print(f"error: {mismatch}", file=sys.stderr)
        return 1

    if args.print_version:
        print(cargo_version)
        return 0

    print(f"rustab version: {cargo_version}")
    print(f"chromium extension: {chrome_manifest['version']}")
    print(f"firefox extension: {firefox_manifest['version']} ({firefox_manifest_id})")
    print(
        "signed firefox xpi: "
        f"{signed_firefox_manifest['version']} ({signed_firefox_id})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
