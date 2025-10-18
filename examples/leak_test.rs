//
// Copyright (c) 2025 Nathan Fiedler
//
use tiered_vector::Vector;

fn test_tiered_vector() {
    // push a bunch, pop nearly all to test compress() and clear()
    let mut array: Vector<usize> = Vector::new();
    for value in 0..1024 {
        array.push(value);
    }
    assert_eq!(array.len(), 1024);
    for _ in 0..1000 {
        array.pop();
    }
    array.clear();

    // test with heap-allocated objects
    let mut array: Vector<String> = Vector::new();
    for _ in 0..1024 {
        let value = ulid::Ulid::new().to_string();
        array.push(value);
    }
    while !array.is_empty() {
        array.pop();
    }

    // IntoIterator: add enough values to allocate a bunch of data blocks
    let mut array: Vector<String> = Vector::new();
    for _ in 0..512 {
        let value = ulid::Ulid::new().to_string();
        array.push(value);
    }
    // skip enough elements to pass over a few data blocks then drop
    for (index, _) in array.into_iter().skip(96).enumerate() {
        if index == 96 {
            // exit the iterator early intentionally
            break;
        }
    }
}

//
// Create and drop collections and iterators in order to test for memory leaks.
// Must allocate Strings in order to fully test the drop implementation.
//
fn main() {
    println!("starting tiered vector testing...");
    test_tiered_vector();
    println!("completed tiered vector testing");
}
