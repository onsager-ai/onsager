#!/usr/bin/env bash
# Sets the version across all npm packages (wrapper + platform-specific).
# Usage: ./scripts/set-npm-version.sh 0.2.0

set -euo pipefail

VERSION="${1:?Usage: set-npm-version.sh <version>}"

PACKAGES=(
  packages/cli/package.json
  npm/cli-darwin-arm64/package.json
  npm/cli-darwin-x64/package.json
  npm/cli-linux-x64/package.json
  npm/cli-linux-arm64/package.json
)

for pkg in "${PACKAGES[@]}"; do
  if [ ! -f "$pkg" ]; then
    echo "Error: expected package file not found: $pkg" >&2
    exit 1
  fi
  # Update both the package version and any optionalDependencies referencing @codervisor/*
  node -e "
      const fs = require('fs');
      const pkg = JSON.parse(fs.readFileSync('$pkg', 'utf8'));
      pkg.version = '$VERSION';
      if (pkg.optionalDependencies) {
        for (const key of Object.keys(pkg.optionalDependencies)) {
          if (key.startsWith('@codervisor/synodic-')) {
            pkg.optionalDependencies[key] = '$VERSION';
          }
        }
      }
      fs.writeFileSync('$pkg', JSON.stringify(pkg, null, 2) + '\n');
    "
    echo "Updated $pkg -> $VERSION"
done
