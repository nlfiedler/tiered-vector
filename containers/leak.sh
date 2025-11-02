#!/bin/sh
#
# Use cargo run to run the code with address santizer enabled. 
#
env RUSTDOCFLAGS=-Zsanitizer=address RUSTFLAGS=-Zsanitizer=address \
    cargo run -Zbuild-std --target x86_64-unknown-linux-gnu --release --example leak_test

env RUSTDOCFLAGS=-Zsanitizer=address RUSTFLAGS=-Zsanitizer=address \
    cargo test -Zbuild-std --target x86_64-unknown-linux-gnu --release
