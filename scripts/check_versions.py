#!/usr/bin/env python3

import argparse
import base64
import binascii
import hashlib
import json
import sys
import tomllib
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = REPO_ROOT / "Cargo.toml"
CHROME_MANIFEST = REPO_ROOT / "extensions" / "chrome" / "manifest.json"
FIREFOX_MANIFEST = REPO_ROOT / "extensions" / "firefox" / "manifest.json"
ORION_MANIFEST = REPO_ROOT / "extensions" / "orion" / "manifest.json"
SIGNED_FIREFOX_XPI_DIR = REPO_ROOT / "extensions" / "firefox-signed"
CHROME_EXTENSION_ID = "nddbmnpippfilnjoebpcnfbpebnllbgo"
FIREFOX_EXTENSION_FILES = [
    "manifest.json",
    "background.js",
    "background_core.js",
    "icon48.png",
    "icon128.png",
]
SHARED_EXTENSION_CORE = REPO_ROOT / "extensions" / "shared" / "background_core.js"


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
    parser.add_argument(
        "--source-only",
        action="store_true",
        help=(
            "Validate only Cargo and source browser manifests. This is useful "
            "early in release workflows before a freshly signed Firefox XPI exists."
        ),
    )
    return parser.parse_args()


def read_json(path: Path) -> dict:
    return json.loads(path.read_text())


def read_workspace_version(path: Path) -> str:
    cargo_toml = tomllib.loads(path.read_text())
    try:
        return cargo_toml["workspace"]["package"]["version"]
    except KeyError as error:
        raise ValueError(
            f"could not find [workspace.package].version in {path}"
        ) from error


def read_signed_firefox_manifest(path: Path) -> dict:
    with zipfile.ZipFile(path) as archive:
        return json.loads(archive.read("manifest.json"))


def signed_firefox_file_bytes(path: Path, relative_name: str) -> bytes:
    with zipfile.ZipFile(path) as archive:
        try:
            return archive.read(relative_name)
        except KeyError as error:
            raise FileNotFoundError(
                f"Signed Firefox XPI is missing {relative_name}"
            ) from error


def firefox_addon_id(manifest: dict) -> str:
    return manifest["browser_specific_settings"]["gecko"]["id"]


def signed_firefox_xpi_path(firefox_manifest: dict) -> Path:
    return SIGNED_FIREFOX_XPI_DIR / f"{firefox_addon_id(firefox_manifest)}.xpi"


def firefox_source_for_signed_payload(relative_name: str) -> tuple[Path, str]:
    if relative_name == "background_core.js":
        return SHARED_EXTENSION_CORE, "extensions/shared/background_core.js"
    return (
        FIREFOX_MANIFEST.parent / relative_name,
        f"extensions/firefox/{relative_name}",
    )


def chrome_extension_id_from_key(manifest_key: str) -> str:
    key_bytes = base64.b64decode(manifest_key, validate=True)
    digest = hashlib.sha256(key_bytes).hexdigest()[:32]
    return "".join(chr(ord("a") + int(nibble, 16)) for nibble in digest)


def main() -> int:
    args = parse_args()

    cargo_version = read_workspace_version(CARGO_TOML)
    chrome_manifest = read_json(CHROME_MANIFEST)
    firefox_manifest = read_json(FIREFOX_MANIFEST)
    orion_manifest = read_json(ORION_MANIFEST)
    firefox_manifest_id = firefox_addon_id(firefox_manifest)
    signed_firefox_xpi = signed_firefox_xpi_path(firefox_manifest)
    signed_firefox_manifest = (
        None if args.source_only else read_signed_firefox_manifest(signed_firefox_xpi)
    )

    observed_versions = {
        "Cargo workspace": cargo_version,
        "Chromium manifest": chrome_manifest["version"],
        "Firefox manifest": firefox_manifest["version"],
        "Orion manifest": orion_manifest["version"],
    }
    if signed_firefox_manifest is not None:
        observed_versions["Signed Firefox XPI"] = signed_firefox_manifest["version"]

    mismatches = [
        f"{label} has version {observed!r}, expected {cargo_version!r}"
        for label, observed in observed_versions.items()
        if observed != cargo_version
    ]

    signed_firefox_id = None

    chrome_manifest_key = chrome_manifest.get("key")
    orion_manifest_key = orion_manifest.get("key")
    if not chrome_manifest_key:
        mismatches.append("Chromium manifest is missing its stable extension key")
    else:
        try:
            chrome_manifest_id = chrome_extension_id_from_key(chrome_manifest_key)
        except binascii.Error as error:
            mismatches.append(f"Chromium manifest key is invalid base64: {error}")
        else:
            if chrome_manifest_id != CHROME_EXTENSION_ID:
                mismatches.append(
                    "Chromium manifest key derives extension id "
                    f"{chrome_manifest_id!r}, expected {CHROME_EXTENSION_ID!r}"
                )

    if not orion_manifest_key:
        mismatches.append("Orion manifest is missing its stable extension key")
    elif chrome_manifest_key and orion_manifest_key != chrome_manifest_key:
        mismatches.append(
            "Orion manifest has key "
            f"{orion_manifest_key!r}, expected Chromium manifest key {chrome_manifest_key!r}"
        )

    if signed_firefox_manifest is not None:
        signed_firefox_id = firefox_addon_id(signed_firefox_manifest)
        if signed_firefox_id != firefox_manifest_id:
            mismatches.append(
                "Signed Firefox XPI has addon id "
                f"{signed_firefox_id!r}, expected {firefox_manifest_id!r}"
            )

        for relative_name in FIREFOX_EXTENSION_FILES:
            source_path, source_label = firefox_source_for_signed_payload(relative_name)
            try:
                signed_file_bytes = signed_firefox_file_bytes(
                    signed_firefox_xpi, relative_name
                )
            except FileNotFoundError as error:
                mismatches.append(str(error))
                continue

            if relative_name == "manifest.json":
                source_json = json.loads(source_path.read_text())
                signed_json = json.loads(signed_file_bytes)
                if signed_json != source_json:
                    mismatches.append(
                        f"Signed Firefox XPI {relative_name} differs from {source_label}"
                    )
                continue

            source_bytes = source_path.read_bytes()
            if signed_file_bytes != source_bytes:
                mismatches.append(
                    f"Signed Firefox XPI {relative_name} differs from {source_label}"
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
    print(f"orion extension: {orion_manifest['version']}")
    if signed_firefox_manifest is not None:
        print(
            "signed firefox xpi: "
            f"{signed_firefox_manifest['version']} ({signed_firefox_id})"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
