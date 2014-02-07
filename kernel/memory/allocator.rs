use core::fail::{abort, out_of_memory};
use core::ptr::offset;
use core::ptr::{set_memory, copy_memory};
use core::i32::ctlz32;

use kernel::ptr::mut_offset;

enum Node {
    UNUSED = 0,
    USED = 1,
    SPLIT = 2,
    FULL = 3
}

pub trait Allocator {
    unsafe fn alloc(&mut self, size: uint) -> (*mut u8, uint);
    unsafe fn zero_alloc(&mut self, size: uint) -> (*mut u8, uint);
    unsafe fn realloc(&mut self, ptr: *mut u8, size: uint) -> (*mut u8, uint);
    unsafe fn free(&mut self, ptr: *mut u8);
}

trait BitvTrait {
    fn get(&self, i: uint) -> Node;
    fn set(&self, i: uint, x: Node);
    fn to_bytes(&self) -> *mut u8;
    fn size(&self) -> uint;
}

static BITV_SIZE: uint = 0x10_000;
pub type BitvStorage = *mut [u32, ..BITV_SIZE / 4];

// vector of 2-bit
pub struct Bitv {
    storage: BitvStorage
}

impl BitvTrait for Bitv {
    #[inline]
    fn get(&self, i: uint) -> Node {
        let w = i / 16;
        let b = (i % 16) * 2;
        unsafe { [UNUSED, USED, SPLIT, FULL][((*self.storage)[w] as uint >> b) & 3] }
    }

    #[inline]
    fn set(&self, i: uint, x: Node) {
        let w = i / 16;
        let b = (i % 16) * 2;
        unsafe { (*self.storage)[w] = (((*self.storage)[w] & !(3 << b)) | (x as u32 << b)); }
    }

    #[inline]
    fn to_bytes(&self) -> *mut u8 {
        self.storage as *mut u8
    }

    #[inline]
    fn size(&self) -> uint {
        BITV_SIZE
    }
}

pub struct BuddyAlloc {
    base: *mut u8,
    order: uint,
    tree: Bitv
}

impl BuddyAlloc {
    pub fn new(base: *mut u8, order: uint, storage: Bitv) -> BuddyAlloc {
        unsafe { set_memory(storage.to_bytes(), 0, storage.size()); }

        BuddyAlloc { base: base, order: order, tree: storage }
    }

    #[inline]
    fn offset(&self, index: uint, level: uint) -> *mut u8 {
        unsafe {
            mut_offset(self.base, (index + 1 - (1 << (self.order - level))) as int << level)
        }
    }
}

impl Allocator for BuddyAlloc {
    fn alloc(&mut self, mut size: uint) -> (*mut u8, uint) {
        if size == 0 {
            size = 1;
        }
        // smallest power of 2 >= size
        let lg2_size = 32 - unsafe { ctlz32(size as i32 - 1) } as uint;

        let mut index = 0; // points to current tree node
        let mut level = self.order; // current height

        loop {
            match (self.tree.get(index), level == lg2_size) {
                (UNUSED, true) => {
                    // Found appropriate unused node
                    self.tree.set(index, USED); // use

                    let mut parent = index;
                    loop {
                        let buddy = parent - 1 + (parent & 1) * 2;
                        match self.tree.get(buddy) {
                            USED | FULL if parent > 0 => {
                                parent = (parent + 1) / 2 - 1;
                                self.tree.set(parent, FULL);
                            }
                            _ => break
                        }
                    }
                    return (
                        self.offset(index, level),
                        1 << lg2_size
                    );
                }
                (UNUSED, false) => {
                    // This large node is unused, split it!
                    self.tree.set(index, SPLIT);
                    self.tree.set(index*2 + 1, UNUSED);
                    self.tree.set(index*2 + 2, UNUSED);
                    index = index * 2 + 1; // left child
                    level -= 1;
                }
                (SPLIT, false) => {
                    // Traverse children
                    index = index * 2 + 1; // left child
                    level -= 1;
                }
                _ => loop {
                    // Go either right or back up
                    if index & 1 == 1 {
                        // right sibling
                        index += 1;
                        break;
                    }

                    // go up by one level
                    level += 1;

                    if index == 0 {
                        // out of memory -- back at tree's root after traversal
                        return (self.base, 0);
                    }

                    index = (index + 1) / 2 - 1; // parent
                }
            }
        }
    }

    fn zero_alloc(&mut self, s: uint) -> (*mut u8, uint) {
        let (ptr, size) = self.alloc(s);
        unsafe { set_memory(ptr, 0, size); }
        (ptr, size)
    }

    fn realloc(&mut self, src: *mut u8, size: uint) -> (*mut u8, uint) {
        self.free(src);
        let (ptr, sz) = self.alloc(size);
        unsafe { copy_memory(ptr, src as *u8, sz); }
        (ptr, sz)
    }

    fn free(&mut self, ptr: *mut u8) {
        let mut length = 1 << self.order;
        let mut left = 0;
        let mut index = 0;

        if ((ptr as uint) < self.base as uint) || (ptr as uint >= self.base as uint + length) {
            return;
        }
        let offset = ptr as uint - self.base as uint;

        loop {
            match self.tree.get(index) {
                UNUSED => return,
                USED => loop {
                    if index == 0 {
                        self.tree.set(0, UNUSED);
                        return;
                    }

                    let buddy = index - 1 + (index & 1) * 2;
                    match self.tree.get(buddy) {
                        UNUSED => {}
                        _ => {
                            self.tree.set(index, UNUSED);
                            loop {
                                let parent = (index + 1) / 2 - 1; // parent
                                match self.tree.get(parent) {
                                    FULL if index > 0 => {
                                        self.tree.set(parent, SPLIT);
                                    }
                                    _ => return
                                }
                                index = parent;
                            }
                        }
                    }
                    index = (index + 1) / 2 - 1; // parent
                },
                _ => {
                    length /= 2;
                    if offset < left + length {
                        index = index * 2 + 1; // left child
                    }
                    else {
                        left += length;
                        index = index * 2 + 2; // right child
                    }
                }
            }
        }
    }
}
