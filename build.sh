#!/usr/bin/env bash
set -eux

cargo build --release --target aarch64-unknown-linux-gnu
