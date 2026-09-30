#!/usr/bin/env bash
# Builds the ParityHarness command-line tool, which compiles the Mac app's own sources.
#
#   parity/harness/build.sh [derived-data-dir]
#
# The derived data folder defaults to build/parity-harness in the repository. PARITY_CONFIGURATION picks the build
# configuration (default Release, the one the app ships with). xcodebuild's log goes to stderr; the only line on
# stdout is the path of the built executable.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
derived="${1:-$root/build/parity-harness}"
configuration="${PARITY_CONFIGURATION:-Release}"

mkdir -p "$derived"
derived="$(cd "$derived" && pwd)"

xcodebuild \
  -project "$root/Compositor.xcodeproj" \
  -scheme ParityHarness \
  -configuration "$configuration" \
  -destination 'platform=macOS' \
  -derivedDataPath "$derived" \
  build >&2

products="$derived/Build/Products/$configuration"
executable="$products/ParityHarness"
if [ ! -x "$executable" ]; then
  echo "build.sh: xcodebuild finished but $executable is missing" >&2
  exit 1
fi
# The tool loads Sparkle.framework from beside itself (LD_RUNPATH_SEARCH_PATHS has @executable_path).
if [ ! -d "$products/Sparkle.framework" ]; then
  echo "build.sh: warning: $products/Sparkle.framework is missing; the tool may fail to launch" >&2
fi
echo "$executable"
