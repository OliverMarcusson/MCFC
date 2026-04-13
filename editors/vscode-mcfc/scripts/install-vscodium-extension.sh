#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
extension_root="$(cd "$script_dir/.." && pwd)"

if ! command -v codium >/dev/null 2>&1; then
  echo "error: codium is not installed or not on PATH" >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "error: npm is not installed or not on PATH" >&2
  exit 1
fi

cd "$extension_root"

echo "==> Installing npm dependencies"
npm install

echo "==> Packaging extension"
npm run package

vsix_path="$(find "$extension_root" -maxdepth 1 -type f -name 'mcfc-syntax-*.vsix' | sort | tail -n 1)"
if [[ -z "$vsix_path" ]]; then
  echo "error: failed to locate packaged VSIX" >&2
  exit 1
fi

echo "==> Installing $vsix_path into VSCodium"
codium --install-extension "$vsix_path" --force

echo "==> Installed extensions"
codium --list-extensions --show-versions | grep '^mcfc\.mcfc-syntax@' || true

echo
echo "Done. Open a .mcf file in VSCodium to activate the extension."
