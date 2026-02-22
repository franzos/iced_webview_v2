#!/usr/bin/env bash
set -euo pipefail

# Strip git-only features, deps and patches from Cargo.toml for publishing.
# crates.io rejects packages with git-only deps — even optional ones.
# Backs up Cargo.toml and restores it after publish (or dry-run).

cp Cargo.toml Cargo.toml.bak
trap 'mv Cargo.toml.bak Cargo.toml' EXIT

# --- Features ---

# Remove blitz feature block (multi-line: "blitz = [" through "]")
sed -i '/^blitz = \[$/,/^\]$/d' Cargo.toml

# Remove servo feature line
sed -i '/^servo = \[/d' Cargo.toml

# --- Dependencies ---

# Blitz git deps
sed -i '/^# Blitz engine deps/d' Cargo.toml
sed -i '/^blitz-dom = {/d' Cargo.toml
sed -i '/^blitz-html = {/d' Cargo.toml
sed -i '/^blitz-paint = {/d' Cargo.toml
sed -i '/^blitz-traits = {/d' Cargo.toml
sed -i '/^blitz-net = {/d' Cargo.toml

# Blitz crates.io deps (only used by blitz feature)
sed -i '/^anyrender = {/d' Cargo.toml
sed -i '/^anyrender_vello_cpu = {/d' Cargo.toml
sed -i '/^peniko = {/d' Cargo.toml
sed -i '/^cursor-icon = {/d' Cargo.toml
sed -i '/^keyboard-types = {/d' Cargo.toml
sed -i '/^tokio = {/d' Cargo.toml

# Servo git dep
sed -i '/^# Servo engine deps/d' Cargo.toml
sed -i '/^servo = {/d' Cargo.toml

# Servo crates.io deps (only used by servo feature)
sed -i '/^rustls = {/d' Cargo.toml
sed -i '/^euclid = {/d' Cargo.toml
sed -i '/^keyboard-types-servo = {/d' Cargo.toml
sed -i '/^dpi = {/d' Cargo.toml

# --- Patch section ---

# Remove patch comment block
sed -i '/^# When both blitz/d' Cargo.toml
sed -i '/^# stylo (crates.io/d' Cargo.toml
sed -i '/^# git rev so they/d' Cargo.toml

# Remove [patch.crates-io] section header and all entries
sed -i '/^\[patch\.crates-io\]$/d' Cargo.toml
sed -i '/^stylo = {/d' Cargo.toml
sed -i '/^stylo_traits = {/d' Cargo.toml
sed -i '/^stylo_atoms = {/d' Cargo.toml
sed -i '/^stylo_dom = {/d' Cargo.toml
sed -i '/^selectors = {/d' Cargo.toml

# --- Cleanup ---

# Collapse runs of blank lines into one
sed -i '/^$/N;/^\n$/d' Cargo.toml

echo "=== Stripped Cargo.toml ==="
cat Cargo.toml
echo "==========================="

if [[ "${1:-}" == "--publish" ]]; then
  cargo publish --allow-dirty
else
  cargo publish --dry-run --allow-dirty
fi
