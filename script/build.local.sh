#!/bin/bash -e

cargo build --release
cp target/${TARGET_TRIPLE}/release/gh-chk .
