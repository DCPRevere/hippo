#!/bin/bash
set -euo pipefail
# Install wasm-pack if not present
if ! command -v wasm-pack &> /dev/null; then
    echo "Installing wasm-pack..."
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
fi
cd "$(dirname "$0")"
wasm-pack build --target web --out-dir ../site/wasm
echo "WASM built → site/wasm/"
