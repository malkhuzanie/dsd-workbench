#!/usr/bin/env bash
# Rebuilds assets/vendor/three.min.js from three-entry.js (needs Node and npm).
# The result is committed, so deploying the site needs no build step.
set -euo pipefail
cd "$(dirname "$0")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp three-entry.js "$work/"
(cd "$work" && npm init -y >/dev/null && npm i --silent three@0.186.1 esbuild)
"$work/node_modules/.bin/esbuild" "$work/three-entry.js" \
  --bundle --minify --format=esm --legal-comments=none \
  --outfile=../assets/vendor/three.min.js
ls -l ../assets/vendor/three.min.js
