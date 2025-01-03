use std::{
    alloc::Layout,
    ffi::c_void,
    fmt::Debug,
    ops::{Deref, DerefMut},
    ptr::null_mut,
};

const INVALID: u32 = u32::MAX;

/// a small pointer that contains some metadata about the allocation
/// otherwise the allocator would need to store this
#[derive(Clone, Copy)]
pub struct FreeListPtr<T> {
    ptr: *mut T,
    pad: u32,
    size: u32,
}

impl<T> FreeListPtr<T> {
    #[must_use]
    pub fn cast<B>(&self) -> FreeListPtr<B> {
        FreeListPtr {
            ptr: self.ptr.cast(),
            pad: self.pad,
            size: self.size,
        }
    }

    #[must_use]
    pub fn as_ptr(&self) -> *mut T {
        self.ptr
    }
}
impl<T> DerefMut for FreeListPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.ptr }
    }
}

impl<T> Deref for FreeListPtr<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr }
    }
}

/// a ``FreeListAllocator`` keeps track of dynamic (de)allocations within a memory region
/// this allocator is affected by memory fragmentation
/// if you want to minimize fragmentation, consider using another allocator.
/// also to improve memory usage the limit of the allocation is ``u32::MAX`` bytes (4.2 GB)
pub struct FreeListAllocator {
    head: u32,
    memory: *mut i8,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    /// the offset to the next node (in bytes)
    next: u32,
    /// the size of this node (in bytes)
    size: u32,
}

impl Node {
    pub unsafe fn touches(node: *const Node, rhs: *const Node) -> bool {
        let node_size = (*node).size as usize;
        let rhs_size = (*node).size as usize;
        node.cast::<i8>().add(node_size) == rhs.cast()
            || rhs.cast::<i8>().add(rhs_size) == node.cast()
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_ptr_alignment)]
impl FreeListAllocator {
    /// # Safety
    /// ``memory`` and ``mem_size`` need to be valid
    /// # Panics
    /// if the size is bigger than ``u32::MAX``
    pub unsafe fn new(memory: *mut i8, mem_size: usize) -> Self {
        assert!(u32::try_from(mem_size).is_ok());
        assert!(memory.is_aligned_to(align_of::<Node>()));

        *memory.cast::<Node>() = Node {
            next: INVALID,
            size: mem_size as u32,
        };

        Self { head: 0, memory }
    }

    /// # Safety
    /// should be deallocated or else it may cause a memory leak
    /// (or is kept until the end of the program)
    /// # Panics
    /// if the memory is smaller then the size of a Node (8 bytes)
    pub unsafe fn allocate(&mut self, layout: Layout) -> Option<FreeListPtr<c_void>> {
        assert!(
            layout.size() >= size_of::<Node>(),
            "allocation needs to have minimum size of {}",
            size_of::<Node>()
        );

        let mut node_index = self.head;
        let mut previous: *mut Node = null_mut();

        while node_index != INVALID {
            let node_addr = self.memory.add(node_index as usize).cast::<Node>();
            let padding = (layout.align() - (node_addr as usize % layout.align())) % layout.align();

            let alloc_size = (layout.size() + padding) as u32;

            let mut return_full_node = |size| {
                let node_to_return;

                if previous.is_null() {
                    node_to_return = self.head as usize;
                    self.head = (*node_addr).next;
                } else {
                    (*previous).next = (*node_addr).next;
                    node_to_return = node_index as usize;
                }

                Some(FreeListPtr {
                    ptr: self.memory.add(node_to_return + padding).cast(),
                    pad: padding as u32,
                    size,
                })
            };

            match (*node_addr).size.cmp(&alloc_size) {
                std::cmp::Ordering::Equal => {
                    return return_full_node(alloc_size);
                }
                std::cmp::Ordering::Greater => {
                    let left_over_size = (*node_addr).size - alloc_size;
                    if left_over_size < size_of::<Node>() as u32 {
                        return return_full_node(alloc_size + left_over_size);
                    }

                    (*node_addr).size -= alloc_size;

                    let new_ptr = node_index + alloc_size;
                    *self.memory.add(new_ptr as usize).cast() = *node_addr;

                    if previous.is_null() {
                        self.head = new_ptr;
                    } else {
                        (*previous).next = new_ptr;
                    }

                    return Some(FreeListPtr {
                        ptr: self.memory.add(node_index as usize + padding).cast(),
                        size: alloc_size,
                        pad: padding as u32,
                    });
                }
                std::cmp::Ordering::Less => {}
            }

            node_index = (*node_addr).next;
            previous = node_addr;
        }

        None
    }

    /// # Safety
    /// invalidates all pointers to this memory block
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::needless_pass_by_value
    )]
    pub unsafe fn dealloc<T>(&mut self, mem: FreeListPtr<T>) {
        // get the actual pointer without padding
        let real_ptr = mem.ptr.offset(-(mem.pad as isize)).cast();
        let mem_size = mem.size + mem.pad;

        *real_ptr = Node {
            size: mem_size,
            next: INVALID,
        };

        // there is no free space, so no point in checking for touching nodes
        if self.head == INVALID {
            self.head = real_ptr.cast::<i8>().offset_from(self.memory) as u32;
        } else {
            self.dealloc_intern(real_ptr);
        }
    }

    #[allow(clippy::cast_sign_loss)]
    unsafe fn dealloc_intern(&mut self, ptr: *mut Node) {
        let mut node_index = self.head;
        let search_index = ptr.cast::<i8>().offset_from(self.memory) as u32;

        let mut p_node: *mut Node = self.memory.add(node_index as usize).cast::<Node>();
        let mut p_previous: *mut Node = null_mut();

        // get the node after and before the deallocation (if exists)
        while node_index < search_index {
            node_index = (*p_node).next;
            p_previous = p_node;
            if node_index == INVALID {
                p_node = null_mut();
            } else {
                p_node = self.memory.add(node_index as usize).cast::<Node>();
            }
        }

        if !p_node.is_null() && Node::touches(p_node, ptr) {
            (*ptr).size += (*p_node).size;
            (*ptr).next = (*p_node).next;
        }

        if p_previous.is_null() {
            self.head = search_index;
        } else if Node::touches(p_previous, ptr) {
            (*p_previous).size += (*ptr).size;
        } else {
            (*p_previous).next = search_index;
        }
    }
}

#[allow(clippy::cast_ptr_alignment)]
impl Debug for FreeListAllocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut list = f.debug_list();

        let mut node_ptr = self.head;

        while node_ptr != INVALID {
            let node = unsafe { &*self.memory.add(node_ptr as usize).cast::<Node>() };
            list.entry(&node.size);
            node_ptr = node.next;
        }

        list.finish()
    }
}
