#!/usr/bin/env bash
set -eux

for b in recorder shutdown_scheduler clock_offset_viewer time_marker timekeeper; do
    for a in aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
        cargo build --bin $b --release --target $a
    done
done
