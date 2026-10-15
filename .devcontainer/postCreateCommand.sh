#!/bin/bash
set -euo pipefail

# Install system dependencies
sudo apt-get update && sudo apt-get install -y lld-22

# cargo-xwin
cargo install cargo-xwin --locked
# cargo xwin cache xwin --update

# Install helpers
cargo install cargo-expand

# Post setup
rustup toolchain install
