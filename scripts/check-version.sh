#!/usr/bin/env bash
# check-version.sh
# Usage:
#   ./scripts/check-version.sh                  # checks current HEAD tag vs Cargo.toml
#   ./scripts/check-version.sh v0.6.0           # checks given tag vs Cargo.toml
#
# Exits 0 if versions match or no tag applies.
# Exits 1 with a human-readable error if they diverge.

set -euo pipefail

CARGO_TOML="${CARGO_TOML:-Cargo.toml}"

# Read the version from workspace Cargo.toml
cargo_version() {
  grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)"/\1/'
}

# Determine which tag to check
if [ $# -ge 1 ]; then
  TAG="$1"
else
  # Look for a v*.*.* tag pointing at HEAD
  TAG=$(git tag --points-at HEAD | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -1 || true)
fi

if [ -z "$TAG" ]; then
  # No release tag on this commit — nothing to validate
  exit 0
fi

TAG_VERSION="${TAG#v}"
CARGO_VERSION=$(cargo_version)

if [ "$TAG_VERSION" != "$CARGO_VERSION" ]; then
  echo "ERROR: Tag '$TAG' does not match version in $CARGO_TOML ('$CARGO_VERSION')."
  echo "  Tag version:   $TAG_VERSION"
  echo "  Cargo version: $CARGO_VERSION"
  echo ""
  echo "Either:"
  echo "  1. Update $CARGO_TOML to version = \"$TAG_VERSION\" and re-tag, or"
  echo "  2. Use the 'Release' workflow_dispatch to bump and tag atomically."
  exit 1
fi

echo "OK: Tag '$TAG' matches Cargo.toml version '$CARGO_VERSION'."
exit 0
