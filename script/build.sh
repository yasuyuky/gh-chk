#!/bin/bash -e

mkdir -p ./dist

TARGET_TRIPLE=${TARGET_TRIPLE:-x86_64-unknown-linux-gnu}
OS_ARCH=${OS_ARCH:-linux-x64}

cargo build --release --target ${TARGET_TRIPLE}
cp target/${TARGET_TRIPLE}/release/gh-chk ./dist/${OS_ARCH}
