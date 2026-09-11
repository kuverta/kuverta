#!/usr/bin/env bash
#
# Copy the shared core and the fuckmail adapter into that app's UI directory.
#
#   fuckbird/tools/install-into.sh apps/desktop/ui
#
# Copied rather than referenced because Tauri serves one directory as the web
# root, and nothing outside `apps/desktop/ui/` is reachable from the page. The
# copy is verbatim and verified with `diff -r`: if it has gone stale, this says
# so rather than letting the app run against files nobody edited.
#
# The layout under the target mirrors this repository exactly —
#
#   <target>/fuckbird/core/...
#   <target>/fuckbird/hosts/fuckmail/...
#
# — for the same reason the Thunderbird manifest sits at the repository root:
# every relative import then resolves unchanged, so the files the app loads are
# byte for byte the files the tests run. Rewriting paths on the way in would be
# a build step, and a build step between the tested code and the shipped code
# is exactly the seam worth not having.

set -euo pipefail

TARGET="${1:-}"
if [[ -z $TARGET ]]; then
    echo "usage: tools/install-into.sh <path-to-ui-directory>" >&2
    echo "   eg: fuckbird/tools/install-into.sh apps/desktop/ui" >&2
    exit 1
fi

# Resolved against the caller's directory, before moving to this repository's
# root — a relative target means what the caller meant by it, not what it would
# happen to mean from here.
if [[ ! -d $TARGET ]]; then
    echo "no such directory: $TARGET" >&2
    exit 1
fi
TARGET="$(cd "$TARGET" && pwd)"

cd "$(dirname "${BASH_SOURCE[0]}")/.."
SOURCE="$PWD"


DEST="$TARGET/fuckbird"

# Removed rather than merged: a file deleted here has to disappear there too,
# and a stale module left behind would still resolve and still run.
rm -rf "$DEST"
mkdir -p "$DEST/hosts"

cp -R "$SOURCE/core" "$DEST/core"
cp -R "$SOURCE/hosts/fuckmail" "$DEST/hosts/fuckmail"

# The copy is only worth anything if it is actually identical.
diff -r "$SOURCE/core" "$DEST/core" >/dev/null
diff -r "$SOURCE/hosts/fuckmail" "$DEST/hosts/fuckmail" >/dev/null

FILES=$(find "$DEST" -type f | wc -l | tr -d ' ')
echo "installed $FILES files into $DEST"
echo "open it at: fuckbird/hosts/fuckmail/ui/triage.html"
