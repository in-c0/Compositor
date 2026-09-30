#!/usr/bin/env bash
# Builds ParityHarness, then renders the parity corpus with it.
#
#   parity/harness/render.sh <out-dir> [--case <id>]...
#
# Writes <out-dir>/<id>.png (and <id>.comp for cases with ops or an imported file) plus <out-dir>/harness-info.json.
# PARITY_CORPUS overrides the corpus folder (default parity/corpus); PARITY_DERIVED_DATA overrides where the tool is
# built (default build/parity-harness). A failing case doesn't fail the script; check harness-info.json.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <out-dir> [--case <id>]..." >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$1"
shift
corpus="${PARITY_CORPUS:-$root/parity/corpus}"
derived="${PARITY_DERIVED_DATA:-$root/build/parity-harness}"

executable="$("$here/build.sh" "$derived")"
"$executable" render --corpus "$corpus" --out "$out" "$@"
