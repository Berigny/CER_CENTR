#!/usr/bin/env sh
# Builds the WebAssembly module and assembles a ready-to-use package in
# crates/evidence-wasm/pkg/ (wrapper, types, package.json, .wasm).
#
#   rustup target add wasm32-unknown-unknown   # once
#   sh tools/build-wasm.sh
set -eu
cd "$(dirname "$0")/.."
cargo build -p evidence-wasm --target wasm32-unknown-unknown --release --locked
out=crates/evidence-wasm/pkg
mkdir -p "$out"
cp target/wasm32-unknown-unknown/release/evidence_wasm.wasm "$out/"
cp crates/evidence-wasm/js/evidence.mjs crates/evidence-wasm/js/evidence.d.ts crates/evidence-wasm/js/package.json "$out/"
cp LICENSE "$out/"
echo "built $out:"
ls -l "$out"
