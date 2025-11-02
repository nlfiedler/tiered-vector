//
// Copyright (c) 2025 Nathan Fiedler
//

//! An implementation of tiered vectors as described in the paper **Tiered
//! Vectors** by Goodrich and Kloss II, published in 1999.
//!
//! * DOI:10.1007/3-540-48447-7_21
//!
//! This implementation is based on the algorithms described in the 1998 version
//! of the paper titled **Tiered Vector** by the same authors. In short, the
//! structure consists of a dope vector that references one or more circular
//! buffers in which all buffers are full, with the exception of the last buffer
//! which may be partially filled.
//!
//! # Memory Usage
//!
//! An empty resizable vector is approximately 72 bytes in size, and while
//! holding elements it will have a space overhead on the order of O(√N) as
//! described in the paper. As elements are added the vector will grow by
//! allocating additional data blocks. Likewise, as elements are removed from
//! the vector, data blocks will be deallocated as they become empty.
//!
//! # Performance
//!
//! The performance and memory layout is as described in the paper: O(√N) space
//! overhead, O(1) get and update operations, and O(√n) insert and remove.
//!
//! # Safety
//!
//! Because this data structure is allocating memory, copying bytes using raw
//! pointers, and de-allocating memory as needed, there are many `unsafe` blocks
//! throughout the code.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::fmt;
use std::ops::{Index, IndexMut};

/// Tiered vector which maintains a collection of circular deques in order to
/// efficiently support insert and remove from any location within the vector.
pub struct Vector<T> {
    /// each deque is of size l = 2^k
    k: usize,
    /// bit-mask to get the index into a circular deque
    k_mask: usize,
    /// the 'l' value (2^k) cached for performance
    l: usize,
    /// when count increases to this size, expand the vector
    upper_limit: usize,
    /// when count decreases to this size, compress the vector
    lower_limit: usize,
    /// number of elements in the vector
    count: usize,
    /// dope vector
    index: Vec<CyclicArray<T>>,
}

impl<T> Vector<T> {
    /// Return an empty vector with zero capacity.
    pub fn new() -> Self {
        // default l value of 4 like std::vec::Vec does for its initial
        // allocation (its initial capacity is zero then becomes 4 then doubles
        // with each expansion)
        Self {
            k: 2,
            k_mask: 3,
            l: 4,
            upper_limit: 16,
            lower_limit: 0,
            count: 0,
            index: vec![],
        }
    }

    /// Double the capacity of this vector by combining its deques into new
    /// deques of double the capacity.
    fn expand(&mut self) {
        let l_prime = 1 << (self.k + 1);
        let old_index: Vec<CyclicArray<T>> = std::mem::take(&mut self.index);
        let mut iter = old_index.into_iter();
        while let Some(a) = iter.next() {
            if let Some(b) = iter.next() {
                self.index.push(CyclicArray::combine(a, b));
            } else {
                self.index.push(CyclicArray::from(l_prime, a));
            }
        }
        self.k += 1;
        self.k_mask = (1 << self.k) - 1;
        self.l = 1 << self.k;
        self.upper_limit = self.l * self.l;
        self.lower_limit = self.upper_limit / 8;
    }

    /// Inserts an element at position `index` within the array, shifting some
    /// elements to the right as needed.
    pub fn insert(&mut self, index: usize, value: T) {
        let len = self.count;
        if index > len {
            panic!("insertion index (is {index}) should be <= len (is {len})");
        }
        if len >= self.upper_limit {
            self.expand();
        }
        if len >= self.capacity() {
            self.index.push(CyclicArray::<T>::new(self.l));
        }
        let sub = index >> self.k;
        let end = len >> self.k;
        let r_prime = index & self.k_mask;
        if sub < end {
            // push-pop phase
            let mut head = self.index[sub].pop_back().unwrap();
            for i in (sub + 1)..end {
                let tail = self.index[i].pop_back().unwrap();
                self.index[i].push_front(head);
                head = tail;
            }
            self.index[end].push_front(head);
        }
        // shift phase
        self.index[sub].insert(r_prime, value);
        self.count += 1;
    }

    /// Appends an element to the back of a collection.
    ///
    /// # Panics
    ///
    /// Panics if a new block is allocated that would exceed `isize::MAX` _bytes_.
    ///
    /// # Time complexity
    ///
    /// O(√N) in the worst case.
    pub fn push(&mut self, value: T) {
        self.insert(self.count, value);
    }

    /// Appends an element if there is sufficient spare capacity, otherwise an
    /// error is returned with the element.
    ///
    /// # Time complexity
    ///
    /// O(√N) in the worst case.
    pub fn push_within_capacity(&mut self, value: T) -> Result<(), T> {
        if self.capacity() <= self.count {
            Err(value)
        } else {
            self.push(value);
            Ok(())
        }
    }

    /// Retrieve a reference to the element at the given offset.
    ///
    /// # Time complexity
    ///
    /// Constant time.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.count {
            None
        } else {
            let sub = index >> self.k;
            let r_prime = index & self.k_mask;
            self.index[sub].get(r_prime)
        }
    }

    /// Returns a mutable reference to an element.
    ///
    /// # Time complexity
    ///
    /// Constant time.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.count {
            None
        } else {
            let sub = index >> self.k;
            let r_prime = index & self.k_mask;
            self.index[sub].get_mut(r_prime)
        }
    }

    /// Shrink the capacity of this vector by splitting its deques into new
    /// deques of half the capacity.
    fn compress(&mut self) {
        let old_index: Vec<CyclicArray<T>> = std::mem::take(&mut self.index);
        for old_deque in old_index.into_iter() {
            let (a, b) = old_deque.split();
            self.index.push(a);
            self.index.push(b);
        }
        self.k -= 1;
        self.k_mask = (1 << self.k) - 1;
        self.l = 1 << self.k;
        self.upper_limit = self.l * self.l;
        self.lower_limit = self.upper_limit / 8;
    }

    /// Removes an element from position `index` within the array, shifting some
    /// elements to the left as needed to close the gap.
    ///
    /// # Time complexity
    ///
    /// O(√N) in the worst case.
    pub fn remove(&mut self, index: usize) -> T {
        let len = self.count;
        if index > len {
            panic!("removal index (is {index}) should be <= len (is {len})");
        }
        // avoid compressing to deques smaller than 4
        if len < self.lower_limit && self.k > 2 {
            self.compress();
        }
        let sub = index >> self.k;
        let end = (len - 1) >> self.k;
        let r_prime = index & self.k_mask;
        // shift phase
        let ret = self.index[sub].remove(r_prime);
        if sub < end {
            // push-pop phase
            let mut tail = self.index[end].pop_front().unwrap();
            for i in (sub + 1..end).rev() {
                let head = self.index[i].pop_front().unwrap();
                self.index[i].push_back(tail);
                tail = head;
            }
            self.index[sub].push_back(tail);
        }
        if self.index[end].is_empty() {
            // prune circular arrays as they become empty
            self.index.pop();
        }
        self.count -= 1;
        ret
    }

    /// Removes the last element from the vector and returns it, or `None` if the
    /// vector is empty.
    ///
    /// # Time complexity
    ///
    /// O(√N) in the worst case.
    pub fn pop(&mut self) -> Option<T> {
        if self.count > 0 {
            Some(self.remove(self.count - 1))
        } else {
            None
        }
    }

    /// Removes and returns the last element from a vector if the predicate
    /// returns true, or `None`` if the predicate returns `false`` or the vector
    /// is empty (the predicate will not be called in that case).
    ///
    /// # Time complexity
    ///
    /// O(√N) in the worst case.
    pub fn pop_if(&mut self, predicate: impl FnOnce(&mut T) -> bool) -> Option<T> {
        if self.count == 0 {
            None
        } else if let Some(last) = self.get_mut(self.count - 1) {
            if predicate(last) { self.pop() } else { None }
        } else {
            None
        }
    }

    // Returns an iterator over the vector.
    //
    // The iterator yields all items from start to end.
    pub fn iter(&self) -> VectorIter<'_, T> {
        VectorIter {
            array: self,
            index: 0,
        }
    }

    /// Return the number of elements in the vector.
    ///
    /// # Time complexity
    ///
    /// Constant time.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns the total number of elements the vector can hold without
    /// reallocating.
    ///
    /// # Time complexity
    ///
    /// Constant time.
    pub fn capacity(&self) -> usize {
        (1 << self.k) * self.index.len()
    }

    /// Returns true if the array has a length of 0.
    ///
    /// # Time complexity
    ///
    /// Constant time.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Clears the vector, removing all values and deallocating all blocks.
    ///
    /// # Time complexity
    ///
    /// O(n) if elements are droppable, otherwise O(√N)
    pub fn clear(&mut self) {
        self.index.clear();
        self.count = 0;
        self.k = 2;
        self.k_mask = 3;
        self.l = 1 << self.k;
        self.upper_limit = self.l * self.l;
        self.lower_limit = self.upper_limit / 8;
    }
}

impl<T> Default for Vector<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Display for Vector<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Vector(k: {}, count: {}, dope: {})",
            self.k,
            self.count,
            self.index.len(),
        )
    }
}

impl<T> Index<usize> for Vector<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        let Some(item) = self.get(index) else {
            panic!("index out of bounds: {}", index);
        };
        item
    }
}

impl<T> IndexMut<usize> for Vector<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let Some(item) = self.get_mut(index) else {
            panic!("index out of bounds: {}", index);
        };
        item
    }
}

impl<A> FromIterator<A> for Vector<A> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let mut arr: Vector<A> = Vector::new();
        for value in iter {
            arr.push(value)
        }
        arr
    }
}

/// Immutable array iterator.
pub struct VectorIter<'a, T> {
    array: &'a Vector<T>,
    index: usize,
}

impl<'a, T> Iterator for VectorIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.array.get(self.index);
        self.index += 1;
        value
    }
}

impl<T> IntoIterator for Vector<T> {
    type Item = T;
    type IntoIter = VectorIntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        let mut me = std::mem::ManuallyDrop::new(self);
        let index = std::mem::take(&mut me.index);
        VectorIntoIter {
            count: me.count,
            index,
        }
    }
}

/// An iterator that moves out of a tiered vector.
pub struct VectorIntoIter<T> {
    /// number of remaining elements
    count: usize,
    /// index of circular deques
    index: Vec<CyclicArray<T>>,
}

impl<T> Iterator for VectorIntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count > 0 {
            let ret = self.index[0].pop_front();
            self.count -= 1;
            if self.index[0].is_empty() {
                self.index.remove(0);
            }
            ret
        } else {
            None
        }
    }
}

/// Basic circular buffer, or what Goodrich and Kloss call a circular deque.
///
/// This implementation allows push and pop from both ends of the buffer and
/// supports insert and remove from arbitrary offsets.
///
/// Unlike the `VecDeque` in the standard library, this array has a fixed size
/// and will panic if a push is performed while the array is already full.
pub struct CyclicArray<T> {
    /// allocated buffer of size `capacity`
    buffer: *mut T,
    /// number of slots allocated in the buffer
    capacity: usize,
    /// offset of the first entry
    head: usize,
    /// number of elements
    count: usize,
}

impl<T> CyclicArray<T> {
    /// Construct a new cyclic array with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let buffer = if capacity == 0 {
            std::ptr::null_mut::<T>()
        } else {
            let layout = Layout::array::<T>(capacity).expect("unexpected overflow");
            unsafe {
                let ptr = alloc(layout).cast::<T>();
                if ptr.is_null() {
                    handle_alloc_error(layout);
                }
                ptr
            }
        };
        Self {
            buffer,
            capacity,
            head: 0,
            count: 0,
        }
    }

    /// Free the buffer for this cyclic array without dropping the elements.
    fn dealloc(&mut self) {
        // apparently this has no effect if capacity is zero
        let layout = Layout::array::<T>(self.capacity).expect("unexpected overflow");
        unsafe {
            dealloc(self.buffer as *mut u8, layout);
        }
    }

    /// Take the elements from the two other cyclic arrays into a new cyclic
    /// array with the combined capacity.
    pub fn combine(a: CyclicArray<T>, b: CyclicArray<T>) -> Self {
        let mut this: CyclicArray<T> = CyclicArray::new(a.capacity + b.capacity);
        let mut this_pos = 0;
        let their_a = std::mem::ManuallyDrop::new(a);
        let their_b = std::mem::ManuallyDrop::new(b);
        for mut other in [their_a, their_b] {
            if other.head + other.count > other.capacity {
                // data wraps around, copy as two blocks
                let src = unsafe { other.buffer.add(other.head) };
                let dst = unsafe { this.buffer.add(this_pos) };
                let count_1 = other.capacity - other.head;
                unsafe { std::ptr::copy(src, dst, count_1) }
                this_pos += count_1;
                let dst = unsafe { this.buffer.add(this_pos) };
                let count_2 = other.count - count_1;
                unsafe { std::ptr::copy(other.buffer, dst, count_2) }
                this_pos += count_2;
            } else {
                // data is contiguous, copy as one block
                let src = unsafe { other.buffer.add(other.head) };
                let dst = unsafe { this.buffer.add(this_pos) };
                unsafe { std::ptr::copy(src, dst, other.count) }
                this_pos += other.count;
            }
            other.dealloc();
            this.count += other.count;
        }
        this
    }

    /// Take the elements from the other cyclic array into a new cyclic array
    /// with the given capacity.
    pub fn from(capacity: usize, other: CyclicArray<T>) -> Self {
        assert!(capacity > other.count, "capacity cannot be less than count");
        let layout = Layout::array::<T>(capacity).expect("unexpected overflow");
        let buffer = unsafe {
            let ptr = alloc(layout).cast::<T>();
            if ptr.is_null() {
                handle_alloc_error(layout);
            }
            ptr
        };
        let mut them = std::mem::ManuallyDrop::new(other);
        if them.head + them.count > them.capacity {
            // data wraps around, copy as two blocks
            let src = unsafe { them.buffer.add(them.head) };
            let count_1 = them.capacity - them.head;
            unsafe { std::ptr::copy(src, buffer, count_1) }
            let dst = unsafe { buffer.add(count_1) };
            let count_2 = them.count - count_1;
            unsafe { std::ptr::copy(them.buffer, dst, count_2) }
        } else {
            // data is contiguous, copy as one block
            let src = unsafe { them.buffer.add(them.head) };
            unsafe { std::ptr::copy(src, buffer, them.count) }
        }
        them.dealloc();
        Self {
            buffer,
            capacity,
            head: 0,
            count: them.count,
        }
    }

    /// Split this cyclic buffer into two equal sized buffers.
    ///
    /// The second buffer may be empty if all elements fit within the first
    /// buffer.
    pub fn split(self) -> (CyclicArray<T>, CyclicArray<T>) {
        assert!(
            self.capacity.is_multiple_of(2),
            "capacity must be an even number"
        );
        let half = self.capacity / 2;
        let mut me = std::mem::ManuallyDrop::new(self);
        let mut a: CyclicArray<T> = CyclicArray::new(half);
        let mut b: CyclicArray<T> = CyclicArray::new(half);
        let mut remaining = me.count;
        for other in [&mut a, &mut b] {
            let mut other_pos = 0;
            while remaining > 0 && !other.is_full() {
                let want_to_copy = if me.head + remaining > me.capacity {
                    me.capacity - me.head
                } else {
                    remaining
                };
                let can_fit = other.capacity - other.count;
                let to_copy = if want_to_copy > can_fit {
                    can_fit
                } else {
                    want_to_copy
                };
                let src = unsafe { me.buffer.add(me.head) };
                let dst = unsafe { other.buffer.add(other_pos) };
                unsafe { std::ptr::copy(src, dst, to_copy) };
                other_pos += to_copy;
                other.count += to_copy;
                me.head = me.physical_add(to_copy);
                remaining -= to_copy;
            }
        }
        me.dealloc();
        (a, b)
    }

    /// Appends an element to the back of the cyclic array.
    ///
    /// # Panic
    ///
    /// Panics if the buffer is already full.
    pub fn push_back(&mut self, value: T) {
        if self.count == self.capacity {
            panic!("cyclic array is full")
        }
        let off = self.physical_add(self.count);
        unsafe { std::ptr::write(self.buffer.add(off), value) }
        self.count += 1;
    }

    /// Prepends an element to the front of the cyclic array.
    ///
    /// # Panic
    ///
    /// Panics if the buffer is already full.
    pub fn push_front(&mut self, value: T) {
        if self.count == self.capacity {
            panic!("cyclic array is full")
        }
        self.head = self.physical_sub(1);
        unsafe { std::ptr::write(self.buffer.add(self.head), value) }
        self.count += 1;
    }

    /// Removes the last element and returns it, or `None` if the cyclic array
    /// is empty.
    pub fn pop_back(&mut self) -> Option<T> {
        if self.count == 0 {
            None
        } else {
            self.count -= 1;
            let off = self.physical_add(self.count);
            unsafe { Some(std::ptr::read(self.buffer.add(off))) }
        }
    }

    /// Removes the first element and returns it, or `None` if the cyclic array
    /// is empty.
    pub fn pop_front(&mut self) -> Option<T> {
        if self.count == 0 {
            None
        } else {
            let old_head = self.head;
            self.head = self.physical_add(1);
            self.count -= 1;
            unsafe { Some(std::ptr::read(self.buffer.add(old_head))) }
        }
    }

    /// Inserts an element at position `index` within the array, possibly
    /// shifting some elements to the left or the right as needed.
    pub fn insert(&mut self, index: usize, value: T) {
        let len = self.count;
        if index > len {
            panic!("insertion index (is {index}) should be <= len (is {len})");
        }
        if len == self.capacity {
            panic!("cyclic array is full")
        }
        //
        // Some free space exists in the array, either on the left, the right,
        // the middle, at both ends, or the entire array is empty. Regardless,
        // there are two cases, shift some elements to the left or to the right.
        //
        let mut r_prime = self.physical_add(index);
        if len > 0 && index < len {
            // need to make space for the new element
            if self.head == 0 || r_prime < self.head {
                // Slide all elements in S,sub of rank greater than or equal to
                // r’ and less than (|S,sub| — r’) mod l to the right by one
                let src = unsafe { self.buffer.add(r_prime) };
                let dst = unsafe { self.buffer.add(r_prime + 1) };
                let count = self.count - index;
                unsafe { std::ptr::copy(src, dst, count) }
            } else {
                // Slide all elements in S,sub of rank less than r’ and greater
                // than or equal to h,sub to the left by one
                let src = unsafe { self.buffer.add(self.head) };
                let count = r_prime - self.head;
                self.head = self.physical_sub(1);
                let dst = unsafe { self.buffer.add(self.head) };
                unsafe { std::ptr::copy(src, dst, count) }
                r_prime -= 1;
            }
        }
        unsafe { std::ptr::write(self.buffer.add(r_prime), value) }
        self.count += 1;
    }

    /// Removes and returns the element at position `index` within the array,
    /// shifting some elements to the left or to the right.
    pub fn remove(&mut self, index: usize) -> T {
        let len = self.count;
        if index >= len {
            panic!("removal index (is {index}) should be < len (is {len})");
        }
        let r_prime = self.physical_add(index);
        let ret = unsafe { std::ptr::read(self.buffer.add(r_prime)) };
        if index < (len - 1) {
            // need to slide elements to fill the new gap
            if self.head == 0 || r_prime < self.head {
                // Slide all elements in S,sub of rank r'+1 to h,sub + |S,sub| to
                // the left by one
                let src = unsafe { self.buffer.add(r_prime + 1) };
                let dst = unsafe { self.buffer.add(r_prime) };
                let count = self.count - index - 1;
                unsafe { std::ptr::copy(src, dst, count) }
            } else {
                // Slide all elements in S,sub of rank greater than or equal to
                // h,sub and less than r' to the right by one
                let src = unsafe { self.buffer.add(self.head) };
                let count = r_prime - self.head;
                self.head = self.physical_add(1);
                let dst = unsafe { self.buffer.add(self.head) };
                unsafe { std::ptr::copy(src, dst, count) }
            }
        }
        self.count -= 1;
        ret
    }

    /// Provides a reference to the element at the given index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.count {
            let idx = self.physical_add(index);
            unsafe { Some(&*self.buffer.add(idx)) }
        } else {
            None
        }
    }

    /// Returns a mutable reference to an element.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.count {
            let idx = self.physical_add(index);
            unsafe { (self.buffer.add(idx)).as_mut() }
        } else {
            None
        }
    }

    /// Clears the cyclic array, removing and dropping all values.
    pub fn clear(&mut self) {
        use std::ptr::{drop_in_place, slice_from_raw_parts_mut};

        if self.count > 0 && std::mem::needs_drop::<T>() {
            let first_slot = self.physical_add(0);
            let last_slot = self.physical_add(self.count);
            if first_slot < last_slot {
                // elements are in one contiguous block
                unsafe {
                    drop_in_place(slice_from_raw_parts_mut(
                        self.buffer.add(first_slot),
                        last_slot - first_slot,
                    ));
                }
            } else {
                // elements wrap around the end of the buffer
                unsafe {
                    drop_in_place(slice_from_raw_parts_mut(
                        self.buffer.add(first_slot),
                        self.capacity - first_slot,
                    ));
                    // check if first and last are at the start of the array
                    if first_slot != last_slot || first_slot != 0 {
                        drop_in_place(slice_from_raw_parts_mut(self.buffer, last_slot));
                    }
                }
            }
        }
        self.head = 0;
        self.count = 0;
    }

    /// Return the number of elements in the array.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns the total number of elements the cyclic array can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns true if the array has a length of 0.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if the array has a length equal to its capacity.
    pub fn is_full(&self) -> bool {
        self.count == self.capacity
    }

    /// Perform a wrapping addition relative to the head of the array and
    /// convert the logical offset to the physical offset within the array.
    fn physical_add(&self, addend: usize) -> usize {
        let logical_index = self.head.wrapping_add(addend);
        if logical_index >= self.capacity {
            logical_index - self.capacity
        } else {
            logical_index
        }
    }

    /// Perform a wrapping subtraction relative to the head of the array and
    /// convert the logical offset to the physical offset within the array.
    fn physical_sub(&self, subtrahend: usize) -> usize {
        let logical_index = self
            .head
            .wrapping_sub(subtrahend)
            .wrapping_add(self.capacity);
        if logical_index >= self.capacity {
            logical_index - self.capacity
        } else {
            logical_index
        }
    }
}

impl<T> Default for CyclicArray<T> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<T> fmt::Display for CyclicArray<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CyclicArray(capacity: {}, head: {}, count: {})",
            self.capacity, self.head, self.count,
        )
    }
}

impl<T> Index<usize> for CyclicArray<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        let Some(item) = self.get(index) else {
            panic!("index out of bounds: {}", index);
        };
        item
    }
}

impl<T> IndexMut<usize> for CyclicArray<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let Some(item) = self.get_mut(index) else {
            panic!("index out of bounds: {}", index);
        };
        item
    }
}

impl<T> Drop for CyclicArray<T> {
    fn drop(&mut self) {
        self.clear();
        self.dealloc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_insert_head() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        for value in (1..=16).rev() {
            sut.insert(0, value);
        }
        assert!(!sut.is_empty());
        for (index, value) in (1..=16).enumerate() {
            assert_eq!(sut[index], value);
        }
    }

    #[test]
    fn test_vector_push_and_clear() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        for value in 0..64 {
            sut.push(value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 64);
        assert_eq!(sut.capacity(), 64);
        for value in 0..64 {
            assert_eq!(sut[value], value);
        }
        sut.clear();
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
    }

    #[test]
    fn test_vector_get_mut() {
        let mut sut = Vector::<usize>::new();
        for value in 0..4 {
            sut.push(value);
        }
        if let Some(value) = sut.get_mut(1) {
            *value = 11;
        } else {
            panic!("get_mut() returned None")
        }
        sut[2] = 12;
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 0);
        assert_eq!(sut[1], 11);
        assert_eq!(sut[2], 12);
        assert_eq!(sut[3], 3);
    }

    #[test]
    fn test_vector_insert_expand() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        for value in (1..=130).rev() {
            sut.insert(0, value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 130);
        assert_eq!(sut.capacity(), 144);
        for value in 0..130 {
            assert_eq!(sut[value], value + 1);
        }
    }

    #[test]
    fn test_vector_push_many() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        for value in 0..100_000 {
            sut.push(value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 100_000);
        assert_eq!(sut.capacity(), 100352);
        for value in 0..100_000 {
            assert_eq!(sut[value], value);
        }
    }

    #[test]
    fn test_vector_push_within_capacity() {
        // empty array has no allocated space
        let mut sut = Vector::<u32>::new();
        assert_eq!(sut.push_within_capacity(101), Err(101));
        sut.push(1);
        sut.push(2);
        assert_eq!(sut.push_within_capacity(3), Ok(()));
        assert_eq!(sut.push_within_capacity(4), Ok(()));
        assert_eq!(sut.push_within_capacity(5), Err(5));
    }

    #[test]
    fn test_vector_remove_small() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        for value in 0..15 {
            sut.push(value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 15);
        for value in 0..15 {
            assert_eq!(sut.remove(0), value);
        }
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
    }

    #[test]
    fn test_vector_remove_medium() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
        for value in 0..2048 {
            sut.push(value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 2048);
        assert_eq!(sut.capacity(), 2048);
        for value in 0..2048 {
            assert_eq!(sut.remove(0), value);
        }
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
    }

    #[test]
    fn test_vector_expand_and_compress() {
        // add enough to cause multiple expansions
        let mut sut = Vector::<usize>::new();
        for value in 0..1024 {
            sut.push(value);
        }
        assert_eq!(sut.len(), 1024);
        assert_eq!(sut.capacity(), 1024);
        // remove enough to cause multiple compressions
        for _ in 0..960 {
            sut.pop();
        }
        // ensure the correct elements remain
        assert_eq!(sut.len(), 64);
        assert_eq!(sut.capacity(), 64);
        for value in 0..64 {
            assert_eq!(sut[value], value);
        }
    }

    #[test]
    fn test_vector_pop_small() {
        let mut sut = Vector::<usize>::new();
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        for value in 0..15 {
            sut.push(value);
        }
        assert!(!sut.is_empty());
        assert_eq!(sut.len(), 15);
        for value in (0..15).rev() {
            assert_eq!(sut.pop(), Some(value));
        }
        assert!(sut.is_empty());
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
    }

    #[test]
    fn test_vector_pop_if() {
        let mut sut = Vector::<u32>::new();
        assert!(sut.pop_if(|_| panic!("should not be called")).is_none());
        for value in 0..10 {
            sut.push(value);
        }
        assert!(sut.pop_if(|_| false).is_none());
        let maybe = sut.pop_if(|v| *v == 9);
        assert_eq!(maybe.unwrap(), 9);
        assert!(sut.pop_if(|v| *v == 9).is_none());
    }

    #[test]
    fn test_vector_iter() {
        let mut sut = Vector::<usize>::new();
        for value in 0..1000 {
            sut.push(value);
        }
        assert_eq!(sut.len(), 1000);
        for (index, value) in sut.iter().enumerate() {
            assert_eq!(sut[index], *value);
        }
    }

    #[test]
    fn test_vector_from_iterator() {
        let mut inputs: Vec<i32> = Vec::new();
        for value in 0..10_000 {
            inputs.push(value);
        }
        let sut: Vector<i32> = inputs.into_iter().collect();
        assert_eq!(sut.len(), 10_000);
        for idx in 0..10_000i32 {
            let maybe = sut.get(idx as usize);
            assert!(maybe.is_some(), "{idx} is none");
            let actual = maybe.unwrap();
            assert_eq!(idx, *actual);
        }
    }

    #[test]
    fn test_vector_into_iterator_drop_empty() {
        let sut: Vector<String> = Vector::new();
        assert_eq!(sut.into_iter().count(), 0);
    }

    #[test]
    fn test_vector_into_iterator_ints_done() {
        let mut sut = Vector::<usize>::new();
        for value in 0..1024 {
            sut.push(value);
        }
        for (idx, elem) in sut.into_iter().enumerate() {
            assert_eq!(idx, elem);
        }
        // sut.len(); // error: ownership of sut was moved
    }

    #[test]
    fn test_vector_remove_insert_basic() {
        let mut sut = Vector::<usize>::new();
        for value in 1..=16 {
            sut.push(value);
        }
        let value = sut.remove(3);
        sut.insert(7, value);
        let mut sorted: Vec<usize> = sut.into_iter().collect();
        sorted.sort();
        for (index, value) in (1..=16).enumerate() {
            assert_eq!(sorted[index], value);
        }
    }

    #[test]
    fn test_vector_random_insert_remove() {
        // trade-off of exhaustive randomized testing and running time
        let mut sut = Vector::<usize>::new();
        let size = 100_000;
        for value in 1..=size {
            sut.push(value);
        }
        for _ in 0..200_000 {
            let from = rand::random_range(0..size);
            let to = rand::random_range(0..size - 1);
            let value = sut.remove(from);
            sut.insert(to, value);
        }
        let mut sorted: Vec<usize> = sut.into_iter().collect();
        sorted.sort();
        for (idx, value) in (1..=size).enumerate() {
            assert_eq!(sorted[idx], value);
        }
    }

    #[test]
    fn test_vector_push_pop_strings() {
        let mut array: Vector<String> = Vector::new();
        for _ in 0..1024 {
            let value = ulid::Ulid::new().to_string();
            array.push(value);
        }
        assert_eq!(array.len(), 1024);
        while let Some(s) = array.pop() {
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn test_cyclic_array_zero_capacity() {
        let sut = CyclicArray::<usize>::new(0);
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 0);
        assert!(sut.is_empty());
        assert!(sut.is_full());
    }

    #[test]
    #[should_panic(expected = "cyclic array is full")]
    fn test_cyclic_array_zero_push_panics() {
        let mut sut = CyclicArray::<usize>::new(0);
        sut.push_back(101);
    }

    #[test]
    fn test_cyclic_array_forward() {
        let mut sut = CyclicArray::<usize>::new(10);
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 10);
        assert!(sut.is_empty());
        assert!(!sut.is_full());

        // add until full
        for value in 0..sut.capacity() {
            sut.push_back(value);
        }
        assert_eq!(sut.len(), 10);
        assert_eq!(sut.capacity(), 10);
        assert!(!sut.is_empty());
        assert!(sut.is_full());

        assert_eq!(sut.get(1), Some(&1));
        assert_eq!(sut[1], 1);
        assert_eq!(sut.get(3), Some(&3));
        assert_eq!(sut[3], 3);
        assert_eq!(sut.get(6), Some(&6));
        assert_eq!(sut[6], 6);
        assert_eq!(sut.get(9), Some(&9));
        assert_eq!(sut[9], 9);
        assert_eq!(sut.get(10), None);

        // remove until empty
        for index in 0..10 {
            let maybe = sut.pop_front();
            assert!(maybe.is_some());
            let value = maybe.unwrap();
            assert_eq!(value, index);
        }
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 10);
        assert!(sut.is_empty());
        assert!(!sut.is_full());
    }

    #[test]
    fn test_cyclic_array_backward() {
        let mut sut = CyclicArray::<usize>::new(10);
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 10);
        assert!(sut.is_empty());
        assert!(!sut.is_full());

        // add until full
        for value in 0..sut.capacity() {
            sut.push_front(value);
        }
        assert_eq!(sut.len(), 10);
        assert_eq!(sut.capacity(), 10);
        assert!(!sut.is_empty());
        assert!(sut.is_full());

        // everything is backwards
        assert_eq!(sut.get(1), Some(&8));
        assert_eq!(sut[1], 8);
        assert_eq!(sut.get(3), Some(&6));
        assert_eq!(sut[3], 6);
        assert_eq!(sut.get(6), Some(&3));
        assert_eq!(sut[6], 3);
        assert_eq!(sut.get(9), Some(&0));
        assert_eq!(sut[9], 0);
        assert_eq!(sut.get(10), None);

        // remove until empty
        for index in 0..10 {
            let maybe = sut.pop_back();
            assert!(maybe.is_some());
            let value = maybe.unwrap();
            assert_eq!(value, index);
        }
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 10);
        assert!(sut.is_empty());
        assert!(!sut.is_full());
    }

    #[test]
    #[should_panic(expected = "index out of bounds:")]
    fn test_cyclic_array_index_out_of_bounds() {
        let mut sut = CyclicArray::<usize>::new(10);
        sut.push_back(10);
        sut.push_back(20);
        let _ = sut[2];
    }

    #[test]
    fn test_cyclic_array_clear_and_reuse() {
        let mut sut = CyclicArray::<String>::new(10);
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        sut.clear();
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        sut.clear();
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        sut.clear();
    }

    #[test]
    fn test_cyclic_array_drop_partial() {
        let mut sut = CyclicArray::<String>::new(10);
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        drop(sut);
    }

    #[test]
    fn test_cyclic_array_drop_full() {
        let mut sut = CyclicArray::<String>::new(10);
        for _ in 0..sut.capacity() {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        drop(sut);
    }

    #[test]
    fn test_cyclic_array_drop_wrapped() {
        let mut sut = CyclicArray::<String>::new(10);
        // push enough to almost fill the buffer
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        // empty the buffer
        while !sut.is_empty() {
            sut.pop_front();
        }
        // push enough to wrap around to the start of the physical buffer
        for _ in 0..7 {
            let value = ulid::Ulid::new().to_string();
            sut.push_back(value);
        }
        drop(sut);
    }

    #[test]
    #[should_panic(expected = "cyclic array is full")]
    fn test_cyclic_array_full_panic() {
        let mut sut = CyclicArray::<usize>::new(1);
        sut.push_back(10);
        sut.push_back(20);
    }

    #[test]
    fn test_cyclic_array_wrapping() {
        let mut sut = CyclicArray::<usize>::new(10);
        // push enough to almost fill the buffer
        for value in 0..7 {
            sut.push_back(value);
        }
        // empty the buffer
        while !sut.is_empty() {
            sut.pop_front();
        }
        // push enough to wrap around to the start of the physical buffer
        for value in 0..7 {
            sut.push_back(value);
        }

        assert_eq!(sut.get(1), Some(&1));
        assert_eq!(sut[1], 1);
        assert_eq!(sut.get(3), Some(&3));
        assert_eq!(sut[3], 3);
        assert_eq!(sut.get(6), Some(&6));
        assert_eq!(sut[6], 6);
        assert_eq!(sut.get(8), None);

        // ensure values are removed correctly
        for value in 0..7 {
            assert_eq!(sut.pop_front(), Some(value));
        }
        assert_eq!(sut.len(), 0);
        assert_eq!(sut.capacity(), 10);
        assert!(sut.is_empty());
        assert!(!sut.is_full());
    }

    #[test]
    fn test_cyclic_array_random_insert_remove() {
        let size = 128;
        let mut sut = CyclicArray::<usize>::new(size);
        for value in 1..=size {
            sut.push_back(value);
        }
        for _ in 0..1024 {
            let from = rand::random_range(0..size);
            let to = rand::random_range(0..size - 1);
            let value = sut.remove(from);
            sut.insert(to, value);
        }
        let mut sorted: Vec<usize> = vec![];
        while let Some(value) = sut.pop_front() {
            sorted.push(value);
        }
        sorted.sort();
        for (idx, value) in (1..=size).enumerate() {
            assert_eq!(sorted[idx], value);
        }
    }

    #[test]
    fn test_cyclic_array_insert_head() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.insert(0, 4);
        sut.insert(0, 3);
        sut.insert(0, 2);
        sut.insert(0, 1);
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
        assert_eq!(sut[2], 3);
        assert_eq!(sut[3], 4);
    }

    #[test]
    fn test_cyclic_array_insert_empty() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // |   |   |   |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 |   |   |   |
        // +---+---+---+---+
        // ```
        sut.insert(0, 1);
        assert_eq!(sut[0], 1);
        assert_eq!(sut.len(), 1);
    }

    #[test]
    fn test_cyclic_array_insert_empty_head_not_zero() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(1);
        sut.push_back(2);
        sut.pop_front();
        sut.pop_front();
        sut.insert(0, 1);
        assert_eq!(sut[0], 1);
        assert_eq!(sut.len(), 1);
    }

    #[test]
    fn test_cyclic_array_insert_loop() {
        let mut sut = CyclicArray::<usize>::new(4);
        for value in 0..100 {
            sut.insert(0, value);
            sut.insert(0, value);
            sut.insert(0, value);
            sut.pop_front();
            sut.pop_front();
            sut.pop_front();
        }
        assert_eq!(sut.len(), 0);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
        assert_eq!(sut[2], 3);
        assert_eq!(sut[3], 4);
    }

    #[test]
    fn test_cyclic_array_insert_1() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 |   |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 | 3 | 2 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.insert(1, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 2);
    }

    #[test]
    fn test_cyclic_array_insert_2() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 2 |   |   | 1 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 3 | 2 |   | 1 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.pop_front();
        sut.pop_front();
        sut.pop_front();
        sut.push_back(2);
        sut.insert(1, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 2);
    }

    #[test]
    fn test_cyclic_array_insert_3() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // |   |   | 1 | 2 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // |   | 1 | 3 | 2 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(2);
        sut.pop_front();
        sut.pop_front();
        sut.insert(1, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 2);
    }

    #[test]
    fn test_cyclic_array_insert_4() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 2 |   |   | 1 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 2 |   | 3 | 1 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.pop_front();
        sut.pop_front();
        sut.pop_front();
        sut.push_back(2);
        sut.insert(0, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 3);
        assert_eq!(sut[1], 1);
        assert_eq!(sut[2], 2);
    }

    #[test]
    fn test_cyclic_array_insert_start() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 |   |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 3 | 1 | 2 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.insert(0, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 3);
        assert_eq!(sut[1], 1);
        assert_eq!(sut[2], 2);
    }

    #[test]
    fn test_cyclic_array_insert_end() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 |   |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.insert(2, 3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
        assert_eq!(sut[2], 3);
    }

    #[test]
    fn test_cyclic_array_insert_end_wrap() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+-V-+---+---+
        // |   | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+-V-+---+---+
        // | 1 | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        sut.pop_front();
        sut.insert(3, 1);
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 4);
        assert_eq!(sut[3], 1);
    }

    #[test]
    #[should_panic(expected = "cyclic array is full")]
    fn test_cyclic_array_insert_full_panic() {
        let mut sut = CyclicArray::<usize>::new(1);
        sut.push_back(10);
        sut.insert(0, 20);
    }

    #[test]
    #[should_panic(expected = "insertion index (is 2) should be <= len (is 0)")]
    fn test_cyclic_array_insert_bounds_panic() {
        let mut sut = CyclicArray::<usize>::new(1);
        sut.insert(2, 20);
    }

    #[test]
    fn test_cyclic_array_remove_start() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 2 | 3 |   |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.remove(0);
        assert_eq!(sut.len(), 2);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
    }

    #[test]
    fn test_cyclic_array_remove_1() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // |   | 1 | 2 | 3 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // |   |   | 1 | 3 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.pop_front();
        sut.remove(1);
        assert_eq!(sut.len(), 2);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 3);
    }

    #[test]
    fn test_cyclic_array_remove_2() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // |   | 1 | 2 | 3 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // |   |   | 2 | 3 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.pop_front();
        sut.remove(0);
        assert_eq!(sut.len(), 2);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
    }

    #[test]
    fn test_cyclic_array_remove_3() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 2 | 3 |   | 1 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 3 |   |   | 1 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.pop_front();
        sut.pop_front();
        sut.pop_front();
        sut.push_back(2);
        sut.push_back(3);
        sut.remove(1);
        assert_eq!(sut.len(), 2);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 3);
    }

    #[test]
    fn test_cyclic_array_remove_start_full() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 2 | 3 | 4 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        sut.remove(0);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 4);
    }

    #[test]
    fn test_cyclic_array_remove_middle_full() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 4 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        sut.remove(2);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
        assert_eq!(sut[2], 4);
    }

    #[test]
    fn test_cyclic_array_remove_end() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 |   |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 | 2 |   |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.remove(2);
        assert_eq!(sut.len(), 2);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
    }

    #[test]
    fn test_cyclic_array_remove_end_full() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+---+---+---+
        // | 1 | 2 | 3 |   |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        sut.remove(3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 2);
        assert_eq!(sut[2], 3);
    }

    #[test]
    fn test_cyclic_array_remove_end_wrap() {
        let mut sut = CyclicArray::<usize>::new(4);
        // start with:
        // ```
        // +---+-V-+---+---+
        // | 5 | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        // becomes:
        // ```
        // +---+-V-+---+---+
        // |   | 2 | 3 | 4 |
        // +---+---+---+---+
        // ```
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        sut.pop_front();
        sut.push_back(5);
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 4);
        assert_eq!(sut[3], 5);
        sut.remove(3);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 2);
        assert_eq!(sut[1], 3);
        assert_eq!(sut[2], 4);
    }

    #[test]
    fn test_cyclic_array_push_pop_remove() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(7);
        sut.push_back(7);
        sut.push_back(7);
        sut.push_back(8);
        sut.pop_front();
        sut.pop_front();
        sut.push_back(10);
        sut.push_back(11);
        sut.remove(2);
        assert_eq!(sut.len(), 3);
        assert_eq!(sut[0], 7);
        assert_eq!(sut[1], 8);
        assert_eq!(sut[2], 11);
    }

    #[test]
    fn test_cyclic_array_push_pop_insert() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(11);
        sut.push_back(11);
        sut.push_back(11);
        sut.push_back(12);
        sut.pop_front();
        sut.pop_front();
        sut.push_back(4);
        sut.insert(2, 3);
        assert_eq!(sut.len(), 4);
        assert_eq!(sut[0], 11);
        assert_eq!(sut[1], 12);
        assert_eq!(sut[2], 3);
        assert_eq!(sut[3], 4);
    }

    #[test]
    #[should_panic(expected = "removal index (is 2) should be < len (is 0)")]
    fn test_cyclic_array_remove_bounds_panic() {
        let mut sut = CyclicArray::<usize>::new(1);
        sut.remove(2);
    }

    #[test]
    fn test_cyclic_array_from_string() {
        let mut sut = CyclicArray::<String>::new(4);
        sut.push_back(ulid::Ulid::new().to_string());
        sut.push_back(ulid::Ulid::new().to_string());
        sut.push_back(ulid::Ulid::new().to_string());
        let copy = CyclicArray::<String>::from(8, sut);
        assert_eq!(copy.len(), 3);
        assert_eq!(copy.capacity(), 8);
        assert!(!copy[0].is_empty());
        assert!(!copy[1].is_empty());
        assert!(!copy[2].is_empty());
    }

    #[test]
    fn test_cyclic_array_from_smaller_1() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        let copy = CyclicArray::<usize>::from(8, sut);
        assert_eq!(copy.len(), 3);
        assert_eq!(copy.capacity(), 8);
        assert_eq!(copy[0], 1);
        assert_eq!(copy[1], 2);
        assert_eq!(copy[2], 3);
    }

    #[test]
    fn test_cyclic_array_from_smaller_2() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(1);
        sut.push_back(2);
        sut.pop_front();
        sut.pop_front();
        sut.push_back(3);
        sut.push_back(4);
        let copy = CyclicArray::<usize>::from(8, sut);
        assert_eq!(copy.len(), 4);
        assert_eq!(copy.capacity(), 8);
        assert_eq!(copy[0], 1);
        assert_eq!(copy[1], 2);
        assert_eq!(copy[2], 3);
        assert_eq!(copy[3], 4);
    }

    #[test]
    fn test_cyclic_array_from_larger_1() {
        let mut sut = CyclicArray::<usize>::new(8);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        let copy = CyclicArray::<usize>::from(4, sut);
        assert_eq!(copy.len(), 3);
        assert_eq!(copy.capacity(), 4);
        assert_eq!(copy[0], 1);
        assert_eq!(copy[1], 2);
        assert_eq!(copy[2], 3);
    }

    #[test]
    fn test_cyclic_array_from_larger_2() {
        let mut sut = CyclicArray::<usize>::new(8);
        for _ in 0..7 {
            sut.push_back(1);
        }
        sut.push_back(2);
        for _ in 0..6 {
            sut.pop_front();
        }
        sut.push_back(3);
        let copy = CyclicArray::<usize>::from(4, sut);
        assert_eq!(copy.len(), 3);
        assert_eq!(copy.capacity(), 4);
        assert_eq!(copy[0], 1);
        assert_eq!(copy[1], 2);
        assert_eq!(copy[2], 3);
    }

    #[test]
    fn test_cyclic_array_combine_string() {
        let mut a = CyclicArray::<String>::new(4);
        a.push_back(ulid::Ulid::new().to_string());
        a.push_back(ulid::Ulid::new().to_string());
        a.push_back(ulid::Ulid::new().to_string());
        let mut b = CyclicArray::<String>::new(4);
        b.push_back(ulid::Ulid::new().to_string());
        b.push_back(ulid::Ulid::new().to_string());
        b.push_back(ulid::Ulid::new().to_string());
        let sut = CyclicArray::combine(a, b);
        assert_eq!(sut.len(), 6);
        assert_eq!(sut.capacity(), 8);
        for i in 0..6 {
            assert!(!sut[i].is_empty());
        }
    }

    #[test]
    fn test_cyclic_array_combine_1_1() {
        let mut a = CyclicArray::<usize>::new(4);
        a.push_back(1);
        a.push_back(2);
        a.push_back(3);
        let mut b = CyclicArray::<usize>::new(4);
        b.push_back(4);
        b.push_back(5);
        b.push_back(6);
        let sut = CyclicArray::combine(a, b);
        assert_eq!(sut.len(), 6);
        assert_eq!(sut.capacity(), 8);
        for i in 0..6 {
            assert_eq!(sut[i], i + 1);
        }
    }

    #[test]
    fn test_cyclic_array_combine_1_2() {
        let mut a = CyclicArray::<usize>::new(4);
        a.push_back(1);
        a.push_back(2);
        a.push_back(3);
        let mut b = CyclicArray::<usize>::new(4);
        b.push_back(4);
        b.push_back(4);
        b.push_back(4);
        b.push_back(5);
        b.pop_front();
        b.pop_front();
        b.push_back(6);
        let sut = CyclicArray::combine(a, b);
        assert_eq!(sut.len(), 6);
        assert_eq!(sut.capacity(), 8);
        for i in 0..6 {
            assert_eq!(sut[i], i + 1);
        }
    }

    #[test]
    fn test_cyclic_array_combine_2_1() {
        let mut a = CyclicArray::<usize>::new(4);
        a.push_back(1);
        a.push_back(1);
        a.push_back(1);
        a.push_back(2);
        a.pop_front();
        a.pop_front();
        a.push_back(3);
        let mut b = CyclicArray::<usize>::new(4);
        b.push_back(4);
        b.push_back(5);
        b.push_back(6);
        let sut = CyclicArray::combine(a, b);
        assert_eq!(sut.len(), 6);
        assert_eq!(sut.capacity(), 8);
        for i in 0..6 {
            assert_eq!(sut[i], i + 1);
        }
    }

    #[test]
    fn test_cyclic_array_combine_2_2() {
        let mut a = CyclicArray::<usize>::new(4);
        a.push_back(1);
        a.push_back(1);
        a.push_back(1);
        a.push_back(2);
        a.pop_front();
        a.pop_front();
        a.push_back(3);
        let mut b = CyclicArray::<usize>::new(4);
        b.push_back(4);
        b.push_back(4);
        b.push_back(4);
        b.push_back(5);
        b.pop_front();
        b.pop_front();
        b.push_back(6);
        let sut = CyclicArray::combine(a, b);
        assert_eq!(sut.len(), 6);
        assert_eq!(sut.capacity(), 8);
        for i in 0..6 {
            assert_eq!(sut[i], i + 1);
        }
    }

    #[test]
    fn test_cyclic_array_split_empty() {
        let big = CyclicArray::<usize>::new(8);
        let (a, b) = big.split();
        assert_eq!(a.len(), 0);
        assert_eq!(a.capacity(), 4);
        assert_eq!(b.len(), 0);
        assert_eq!(b.capacity(), 4);
    }

    #[test]
    fn test_cyclic_array_split_string() {
        let mut big = CyclicArray::<String>::new(8);
        for _ in 0..8 {
            big.push_back(ulid::Ulid::new().to_string());
        }
        let (a, b) = big.split();
        assert_eq!(a.len(), 4);
        assert_eq!(a.capacity(), 4);
        assert!(!a[0].is_empty());
        assert!(!a[1].is_empty());
        assert!(!a[2].is_empty());
        assert!(!a[3].is_empty());
        assert_eq!(b.len(), 4);
        assert_eq!(b.capacity(), 4);
        assert!(!b[0].is_empty());
        assert!(!b[1].is_empty());
        assert!(!b[2].is_empty());
        assert!(!b[3].is_empty());
    }

    #[test]
    fn test_cyclic_array_split_full() {
        let mut big = CyclicArray::<usize>::new(8);
        for value in 1..=8 {
            big.push_back(value);
        }
        let (a, b) = big.split();
        assert_eq!(a.len(), 4);
        assert_eq!(a.capacity(), 4);
        assert_eq!(a[0], 1);
        assert_eq!(a[1], 2);
        assert_eq!(a[2], 3);
        assert_eq!(a[3], 4);
        assert_eq!(b.len(), 4);
        assert_eq!(b.capacity(), 4);
        assert_eq!(b[0], 5);
        assert_eq!(b[1], 6);
        assert_eq!(b[2], 7);
        assert_eq!(b[3], 8);
    }

    #[test]
    fn test_cyclic_array_split_partial_whole() {
        let mut big = CyclicArray::<usize>::new(8);
        for value in 1..=6 {
            big.push_back(value);
        }
        let (a, b) = big.split();
        assert_eq!(a.len(), 4);
        assert_eq!(a.capacity(), 4);
        assert_eq!(a[0], 1);
        assert_eq!(a[1], 2);
        assert_eq!(a[2], 3);
        assert_eq!(a[3], 4);
        assert_eq!(b.len(), 2);
        assert_eq!(b.capacity(), 4);
        assert_eq!(b[0], 5);
        assert_eq!(b[1], 6);
    }

    #[test]
    fn test_cyclic_array_split_partial_split() {
        let mut big = CyclicArray::<usize>::new(8);
        for value in 1..=6 {
            big.push_back(value);
        }
        big.pop_front();
        big.pop_front();
        big.pop_front();
        big.push_back(7);
        big.push_back(8);
        big.push_back(9);
        let (a, b) = big.split();
        assert_eq!(a.len(), 4);
        assert_eq!(a.capacity(), 4);
        assert_eq!(a[0], 4);
        assert_eq!(a[1], 5);
        assert_eq!(a[2], 6);
        assert_eq!(a[3], 7);
        assert_eq!(b.len(), 2);
        assert_eq!(b.capacity(), 4);
        assert_eq!(b[0], 8);
        assert_eq!(b[1], 9);
    }

    #[test]
    fn test_cyclic_array_get_mut() {
        let mut sut = CyclicArray::<usize>::new(4);
        sut.push_back(1);
        sut.push_back(2);
        sut.push_back(3);
        sut.push_back(4);
        if let Some(value) = sut.get_mut(1) {
            *value = 12;
        } else {
            panic!("get_mut() returned None")
        }
        sut[2] = 13;
        assert_eq!(sut[0], 1);
        assert_eq!(sut[1], 12);
        assert_eq!(sut[2], 13);
        assert_eq!(sut[3], 4);
    }
}
