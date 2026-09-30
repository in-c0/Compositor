#!/usr/bin/env bash
# Renders the UI states with the Mac app's own views, and captures its menus.
#
#   parity/harness/ui.sh <out-dir> [--state <id>]...
#
# Writes <out-dir>/<id>.png for each state in parity/ui/states.toml (or only the named ones), <out-dir>/ui-info.json
# and <out-dir>/menus.json. The main menu comes from the real app (menus/dump-main-menu.sh); set
# PARITY_SKIP_MAIN_MENU=1 to skip building the app, and menus.json says the main menu is missing. PARITY_UI_STATES,
# PARITY_CORPUS, PARITY_DERIVED_DATA and PARITY_APP_DERIVED_DATA override the defaults. A state that fails doesn't
# fail the script; check ui-info.json.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <out-dir> [--state <id>]..." >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$1"
shift
states="${PARITY_UI_STATES:-$root/parity/ui/states.toml}"
corpus="${PARITY_CORPUS:-$root/parity/corpus}"
derived="${PARITY_DERIVED_DATA:-$root/build/parity-harness}"
app_derived="${PARITY_APP_DERIVED_DATA:-$root/build/parity-app}"

mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

menu="$work/main-menu.json"
if [ "${PARITY_SKIP_MAIN_MENU:-}" != "1" ]; then
  if ! "$here/menus/dump-main-menu.sh" "$menu" "$app_derived" > /dev/null; then
    echo "ui.sh: the main menu wasn't captured; menus.json will say so" >&2
  fi
fi

executable="$("$here/build.sh" "$derived")"
"$executable" ui --states "$states" --corpus "$corpus" --out "$out" --main-menu "$menu" "$@"
