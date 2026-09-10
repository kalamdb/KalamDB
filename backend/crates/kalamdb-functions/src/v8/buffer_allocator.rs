//! ArrayBuffer backing stores are not covered by V8's managed-heap limit.

use std::{
    alloc::{alloc, alloc_zeroed, dealloc, Layout},
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(crate) struct BufferAllocator {
    used:  AtomicUsize,
    limit: usize,
}

impl BufferAllocator {
    pub(crate) fn create(limit: usize) -> v8::UniqueRef<v8::Allocator> {
        let state = Box::into_raw(Box::new(Self {
            used: AtomicUsize::new(0),
            limit,
        }));
        // SAFETY: V8 owns this allocation and calls the matching destructor exactly once.
        unsafe { v8::new_rust_allocator(state, &VTABLE) }
    }

    fn allocate(&self, len: usize, zeroed: bool) -> *mut c_void {
        let Ok(layout) = Layout::from_size_align(len.max(1), 16) else {
            return ptr::null_mut();
        };
        if self
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(len).filter(|next| *next <= self.limit)
            })
            .is_err()
        {
            return ptr::null_mut();
        }
        // SAFETY: layout has nonzero size and valid alignment; free uses the same layout.
        let data = unsafe {
            if zeroed {
                alloc_zeroed(layout)
            } else {
                alloc(layout)
            }
        };
        if data.is_null() {
            self.used.fetch_sub(len, Ordering::AcqRel);
        }
        data.cast()
    }
}

unsafe extern "C" fn allocate(state: &BufferAllocator, len: usize) -> *mut c_void {
    state.allocate(len, true)
}
unsafe extern "C" fn allocate_uninitialized(state: &BufferAllocator, len: usize) -> *mut c_void {
    state.allocate(len, false)
}
unsafe extern "C" fn free(state: &BufferAllocator, data: *mut c_void, len: usize) {
    if !data.is_null() {
        // SAFETY: V8 returns a pointer and length allocated by this allocator.
        unsafe {
            dealloc(data.cast(), Layout::from_size_align_unchecked(len.max(1), 16));
        }
        state.used.fetch_sub(len, Ordering::AcqRel);
    }
}
unsafe extern "C" fn destroy(state: *const BufferAllocator) {
    // SAFETY: V8 owns the unique Box transferred by create().
    unsafe {
        drop(Box::from_raw(state.cast_mut()));
    }
}
static VTABLE: v8::RustAllocatorVtable<BufferAllocator> = v8::RustAllocatorVtable {
    allocate,
    allocate_uninitialized,
    free,
    drop: destroy,
};
