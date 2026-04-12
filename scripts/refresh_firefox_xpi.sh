#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
credentials_path="${HOME}/.web-ext-credentials"

usage() {
  cat <<'EOF'
Usage: refresh-firefox-xpi [--credentials PATH]

Sign Rustab's Firefox extension with AMO unlisted credentials and refresh the
checked-in signed XPI at extensions/firefox-signed/rustab@rustab.dev.xpi.

Credentials may be provided either through WEB_EXT_API_KEY / WEB_EXT_API_SECRET
already set in the environment or through a shell-style credentials file.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --credentials)
      credentials_path="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [ -z "${WEB_EXT_API_KEY:-}" ] || [ -z "${WEB_EXT_API_SECRET:-}" ]; then
  if [ ! -f "$credentials_path" ]; then
    echo "missing credentials: set WEB_EXT_API_KEY / WEB_EXT_API_SECRET or provide $credentials_path" >&2
    exit 1
  fi

  # shellcheck disable=SC1090
  . "$credentials_path"
fi

if [ -z "${WEB_EXT_API_KEY:-}" ] || [ -z "${WEB_EXT_API_SECRET:-}" ]; then
  echo "WEB_EXT_API_KEY and WEB_EXT_API_SECRET are required after loading credentials" >&2
  exit 1
fi

cd "$repo_root"
rm -rf web-ext-artifacts

web-ext sign \
  --source-dir=extensions/firefox \
  --channel=unlisted \
  --api-key="$WEB_EXT_API_KEY" \
  --api-secret="$WEB_EXT_API_SECRET"

mkdir -p extensions/firefox-signed
cp web-ext-artifacts/*.xpi extensions/firefox-signed/rustab@rustab.dev.xpi
python3 scripts/check_versions.py

echo "refreshed extensions/firefox-signed/rustab@rustab.dev.xpi"
