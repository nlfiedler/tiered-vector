//
// Copyright (c) 2025 Nathan Fiedler
//
use std::time::Instant;
use tiered_vector::Vector;
// use pcramer::VecTiered;

fn benchmark_tiered_vector(coll: &mut Vector<usize>, size: usize, ops: usize) {
    let start = Instant::now();
    for value in 0..size {
        coll.push(value);
    }
    let duration = start.elapsed();
    println!("tiered create: {:?}", duration);

    // test sequenced access for entire collection
    let start = Instant::now();
    for (index, value) in coll.iter().enumerate() {
        assert_eq!(*value, index);
    }
    let duration = start.elapsed();
    println!("tiered ordered: {:?}", duration);

    // test random remove and insert operations
    let start = Instant::now();
    for _ in 0..ops {
        let from = rand::random_range(0..size);
        let to = rand::random_range(0..size - 1);
        let value = coll.remove(from);
        coll.insert(to, value);
    }
    let duration = start.elapsed();
    println!("tiered {ops} remove/insert: {:?}", duration);

    // test popping all elements from the array
    let unused = coll.capacity() - coll.len();
    println!("unused capacity: {unused}");
    let start = Instant::now();
    while !coll.is_empty() {
        coll.pop();
    }
    let duration = start.elapsed();
    println!("tiered pop-all: {:?}", duration);
    println!("tiered capacity: {}", coll.capacity());
}

//
// fails the iteration test
//
// fn benchmark_vectiered(coll: &mut VecTiered<usize>, size: usize, ops: usize) {
//     let start = Instant::now();
//     for value in 0..size {
//         coll.push(value);
//     }
//     let duration = start.elapsed();
//     println!("vectiered create: {:?}", duration);

//     // test sequenced access for entire collection
//     let start = Instant::now();
//     for (index, value) in coll.iter().enumerate() {
//         assert_eq!(*value, index);
//     }
//     let duration = start.elapsed();
//     println!("vectiered ordered: {:?}", duration);

//     // test random remove and insert operations
//     let start = Instant::now();
//     for _ in 0..ops {
//         let from = rand::random_range(0..size);
//         let to = rand::random_range(0..size - 1);
//         let value = coll.remove(from);
//         coll.insert(to, value);
//     }
//     let duration = start.elapsed();
//     println!("vectiered {ops} remove/insert: {:?}", duration);

//     // test popping all elements from the array
//     let unused = coll.capacity() - coll.len();
//     println!("unused capacity: {unused}");
//     let start = Instant::now();
//     while !coll.is_empty() {
//         coll.pop();
//     }
//     let duration = start.elapsed();
//     println!("vectiered pop-all: {:?}", duration);
//     println!("vectiered capacity: {}", coll.capacity());
// }

fn benchmark_vector(size: usize, ops: usize) {
    let start = Instant::now();
    let mut coll: Vec<usize> = Vec::new();
    for value in 0..size {
        coll.push(value);
    }
    let duration = start.elapsed();
    println!("vector create: {:?}", duration);

    // test sequenced access for entire collection
    let start = Instant::now();
    for (index, value) in coll.iter().enumerate() {
        assert_eq!(*value, index);
    }
    let duration = start.elapsed();
    println!("vector ordered: {:?}", duration);

    // test random remove and insert operations
    let start = Instant::now();
    for _ in 0..ops {
        let from = rand::random_range(0..size);
        let to = rand::random_range(0..size - 1);
        let value = coll.remove(from);
        coll.insert(to, value);
    }
    let duration = start.elapsed();
    println!("vector {ops} remove/insert: {:?}", duration);

    // test popping all elements from the vector
    let unused = coll.capacity() - coll.len();
    println!("unused capacity: {unused}");
    let start = Instant::now();
    while !coll.is_empty() {
        coll.pop();
    }
    let duration = start.elapsed();
    println!("vector pop-all: {:?}", duration);
    println!("vector capacity: {}", coll.capacity());
}

fn main() {
    let size = 100_000_000;
    println!("creating Tiered Vector of {size} elements...");
    let mut coll: Vector<usize> = Vector::new();
    benchmark_tiered_vector(&mut coll, size, 200_000);
    // let size = 1_000_000;
    // println!("creating VecTiered of {size} elements...");
    // let mut coll: VecTiered<usize> = VecTiered::with_capacity(size);
    // benchmark_vectiered(&mut coll, size, 2_000);
    let size = 5_000_000;
    println!("creating Vec of {size} elements...");
    benchmark_vector(size, 20_000);
}
