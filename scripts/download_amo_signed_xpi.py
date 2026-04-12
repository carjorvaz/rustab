#!/usr/bin/env python3

import argparse
import base64
import hashlib
import hmac
import json
import os
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

AMO_API_BASE = "https://addons.mozilla.org/api/v5/addons/addon"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Download an existing public signed Firefox XPI for a specific AMO "
            "addon/version using AMO API credentials."
        )
    )
    parser.add_argument("--addon-id", required=True, help="Firefox addon id, for example rustab@rustab.dev")
    parser.add_argument("--version", required=True, help="Version to download, for example 0.1.1")
    parser.add_argument("--out", required=True, type=Path, help="Where to write the downloaded XPI")
    return parser.parse_args()


def require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        print(f"missing required environment variable: {name}", file=sys.stderr)
        raise SystemExit(1)
    return value


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def auth_header(api_key: str, api_secret: str) -> str:
    now = int(time.time())
    header = {"alg": "HS256", "typ": "JWT"}
    payload = {"iss": api_key, "iat": now, "exp": now + 300}
    segments = [
        b64url(json.dumps(header, separators=(",", ":")).encode()),
        b64url(json.dumps(payload, separators=(",", ":")).encode()),
    ]
    message = ".".join(segments).encode()
    signature = hmac.new(api_secret.encode(), message, hashlib.sha256).digest()
    return "JWT " + ".".join([segments[0], segments[1], b64url(signature)])


def fetch_json(url: str, authorization: str) -> dict:
    request = urllib.request.Request(
        url,
        headers={
            "Authorization": authorization,
            "Accept": "application/json",
            "User-Agent": "rustab-release-helper",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def download_file(url: str, authorization: str, out: Path) -> None:
    request = urllib.request.Request(
        url,
        headers={
            "Authorization": authorization,
            "User-Agent": "rustab-release-helper",
        },
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        out.write_bytes(response.read())


def main() -> int:
    args = parse_args()
    api_key = require_env("WEB_EXT_API_KEY")
    api_secret = require_env("WEB_EXT_API_SECRET")
    authorization = auth_header(api_key, api_secret)
    addon_id = urllib.parse.quote(args.addon_id, safe="")
    version_url = f"{AMO_API_BASE}/{addon_id}/versions/{args.version}/"
    version_data = fetch_json(version_url, authorization)
    file_info = version_data.get("file") or {}
    status = file_info.get("status")
    file_url = file_info.get("url")

    if status != "public" or not file_url:
        print(
            f"AMO version {args.version} for {args.addon_id} is not downloadable yet "
            f"(status={status!r})",
            file=sys.stderr,
        )
        return 1

    args.out.parent.mkdir(parents=True, exist_ok=True)
    download_file(file_url, authorization, args.out)
    print(f"downloaded {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
