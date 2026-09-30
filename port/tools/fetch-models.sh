#!/usr/bin/env bash
# Fetches the model Remove Background uses in the port (U²-Netp, Apache-2.0, from rembg's
# releases) into models/ next to the port's executables, checked against its SHA-256. The engine
# looks for it there (or in $COMPOSITOR_MODELS); without it Remove Background is unsupported.
#
#   bash port/tools/fetch-models.sh [dir...]     (default: port/target/release port/target/debug)

set -euo pipefail
url="https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx"
sha256="309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8"

cache="${TMPDIR:-${TEMP:-/tmp}}/compositor-models"
file="$cache/u2netp.onnx"
digest() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
mkdir -p "$cache"
if [ ! -f "$file" ] || [ "$(digest "$file")" != "$sha256" ]; then
    curl -fsSL "$url" -o "$file"
fi
actual="$(digest "$file")"
if [ "$actual" != "$sha256" ]; then
    rm -f "$file"
    echo "u2netp.onnx has SHA-256 $actual, expected $sha256" >&2
    exit 1
fi
[ $# -gt 0 ] || set -- port/target/release port/target/debug
for dir in "$@"; do
    mkdir -p "$dir/models"
    cp "$file" "$dir/models/"
    echo "u2netp.onnx -> $dir/models"
done
