#!/usr/bin/env bash
# One-time setup: point git at .githooks for this repo.
set -euo pipefail
git config core.hooksPath .githooks
echo "Git hooks configured. Pre-push version check is now active."
