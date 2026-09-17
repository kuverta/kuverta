#!/usr/bin/env bash
#
# Prepare a release: set the version, commit, and tag it.
#
#   tools/release.sh 0.2.0
#
# Nothing is pushed. Pushing the tag starts .github/workflows/release.yml,
# which builds the app and leaves a draft release on GitHub to publish.
# See docs/releasing.md.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# cargo is not on PATH in a non-login shell, which is what make starts.
export PATH="$HOME/.cargo/bin:$PATH"

VERSION="${1:-}"
if [[ ! $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "usage: tools/release.sh <version>, e.g. 0.2.0" >&2
    exit 1
fi
TAG="v$VERSION"

if [[ -n $(git status --porcelain) ]]; then
    echo "the working tree has changes; commit or stash them first" >&2
    exit 1
fi
BRANCH=$(git branch --show-current)
if [[ $BRANCH != main ]]; then
    echo "releases are made from main, and this is $BRANCH" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    echo "$TAG already exists" >&2
    exit 1
fi

CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
echo "version $CURRENT -> $VERSION"

# The workspace version is the one every crate and the app report.
# The first `version = ` line is [workspace.package]'s; perl because sed's
# "first match only" differs between macOS and Linux.
VERSION="$VERSION" perl -0pi -e 's/^version = "[^"]*"/version = "$ENV{VERSION}"/m' Cargo.toml
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null

git add Cargo.toml Cargo.lock
# The first release can be of the version already in Cargo.toml.
if git diff --cached --quiet; then
    echo "Cargo.toml already says $VERSION; tagging the current commit"
else
    git commit -m "Release $VERSION"
fi
git tag -a "$TAG" -m "kuverta $VERSION"

echo
echo "Tagged $TAG. To build it on GitHub:"
echo "  git push origin main $TAG"
