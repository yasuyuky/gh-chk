#!/usr/bin/env bash
set -e

# Build
cargo build --release

# Set gh CLI extension directory
GH_EXT_DIR="$HOME/.local/share/gh/extensions"

# Create gh-chk extension directory
mkdir -p "$GH_EXT_DIR/gh-chk"

# Copy executable
cp -f target/release/gh-chk "$GH_EXT_DIR/gh-chk"

# Add execution permission
chmod +x "$GH_EXT_DIR/gh-chk/gh-chk"

echo "Installation complete: Check with 'gh extension list'"
