#!/usr/bin/env bash
# Captures the real app's main menu bar as JSON.
#
#   parity/harness/menus/dump-main-menu.sh <out-file> [derived-data-dir]
#
# The menus are SwiftUI commands in CompositorApp.swift, which ParityHarness can't compile (the harness has its own
# main), so the app itself is built and launched with MenuDump.m inserted by dyld. Once the app has launched, the
# library writes NSApp.mainMenu to standard output and exits the app. No app source is changed: the build is the
# Debug configuration (ad-hoc signed, as the project sets it) with the hardened runtime off, because dyld ignores
# DYLD_INSERT_LIBRARIES for hardened apps.
#
# The derived data folder defaults to build/parity-app. PARITY_MENUS_DELAY sets how long after launch the menu is read
# (default 3 seconds). Exits non-zero, with the app's log on stderr, when no menu was captured.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <out-file> [derived-data-dir]" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$1"
derived="${2:-$root/build/parity-app}"

mkdir -p "$derived" "$(dirname "$out")"
derived="$(cd "$derived" && pwd)"
rm -f "$out"

xcodebuild \
  -project "$root/Compositor.xcodeproj" \
  -scheme Compositor \
  -configuration Debug \
  -destination 'platform=macOS' \
  -derivedDataPath "$derived" \
  ENABLE_HARDENED_RUNTIME=NO \
  build >&2

executable="$derived/Build/Products/Debug/Compositor.app/Contents/MacOS/Compositor"
if [ ! -x "$executable" ]; then
  echo "dump-main-menu.sh: xcodebuild finished but $executable is missing" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
clang -dynamiclib -fobjc-arc -framework AppKit -o "$work/libParityMenuDump.dylib" "$here/MenuDump.m" >&2

# Straight from the executable, not through `open`, so its standard output comes here. The saved-state switch keeps a
# previous run's windows from being restored.
set +e
DYLD_INSERT_LIBRARIES="$work/libParityMenuDump.dylib" "$executable" -ApplePersistenceIgnoreState YES \
  > "$work/stdout.txt" 2> "$work/stderr.txt" &
pid=$!
for _ in $(seq 1 120); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done
if kill -0 "$pid" 2>/dev/null; then
  echo "dump-main-menu.sh: the app was still running after 120 s; stopping it" >&2
  kill -9 "$pid" 2>/dev/null
fi
wait "$pid"
status=$?
set -e

sed -n '/^PARITY-MENUS-BEGIN$/,/^PARITY-MENUS-END$/p' "$work/stdout.txt" | sed '1d;$d' > "$out"
if [ ! -s "$out" ]; then
  rm -f "$out"
  echo "dump-main-menu.sh: the app exited with status $status without reporting its menu. Its log:" >&2
  tail -n 60 "$work/stderr.txt" >&2
  tail -n 20 "$work/stdout.txt" >&2
  exit 1
fi
echo "$out"
