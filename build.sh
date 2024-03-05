#!/usr/bin/env bash
set -eux

cargo build --bin recorder --profile=release-with-debug --target aarch64-unknown-linux-gnu
cargo build --bin shutdown_scheduler --profile=release-with-debug --target aarch64-unknown-linux-gnu
