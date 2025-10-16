# Tiered Vectors

## Overview

This Rust crate provides an implementation of a tiered vector as described in the paper **Tiered Vector** by Goodrich and Kloss II, published in 1998 (and subsequently in 1999).

* https://www.researchgate.net/publication/225174363_Tiered_Vectors_Efficient_Dynamic_Arrays_for_Rank-Based_Sequences

This data structure supports efficient get and update operations with a running time of O(1) as well as insert and remove operations on the order of O(√N). It uses a collection of circular buffers to achieve this with a space overhead on the order of O(√N).

## Examples

A simple example copied from the unit tests.

```rust
let mut tiered = Vector::<usize>::new();
assert!(tiered.is_empty());
for value in (1..=16).rev() {
    tiered.insert(0, value);
}
assert!(!tiered.is_empty());
for (index, value) in (1..=16).enumerate() {
    assert_eq!(tiered[index], value);
}
```

## Supported Rust Versions

The Rust edition is set to `2024` and hence version `1.85.0` is the minimum supported version.

## Troubleshooting

### Memory Leaks

Finding memory leaks with [Address Sanitizer](https://clang.llvm.org/docs/AddressSanitizer.html) is fairly [easy](https://doc.rust-lang.org/beta/unstable-book/compiler-flags/sanitizer.html) and seems to work best on Linux. The shell script below gives a quick demonstration of running one of the examples with ASAN analysis enabled.

```shell
#!/bin/sh
env RUSTDOCFLAGS=-Zsanitizer=address RUSTFLAGS=-Zsanitizer=address \
    cargo run -Zbuild-std --target x86_64-unknown-linux-gnu --release --example leak_test
```

## References

* \[1\]: [Tiered Vectors (1998)](https://cs.brown.edu/cgc/jdsl/papers/tiered-vector.pdf)
    - There is a 1999 version that lacks psuedo-code but is largely the same.
