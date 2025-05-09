#!/bin/bash

# Check if rustup is installed
if ! command -v rustup &> /dev/null
then
    echo "rustup not found. Installing Rust..."
    # Install rustup - this is the recommended way to install Rust
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    # Source cargo environment for the current shell session
    source "$HOME/.cargo/env"
else
    echo "rustup is already installed."
    # Ensure cargo environment is sourced
    source "$HOME/.cargo/env"
fi


# Build the project
echo "Building the project..."
cargo build --release

echo "Starting Monitor..."
./target/release/avs_boost_monitor