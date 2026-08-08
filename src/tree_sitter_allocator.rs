// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

//! Route Tree-sitter allocations through Rust's WebAssembly allocator.
//!
//! `tree-sitter-language` supplies a small, resettable C allocator for
//! `wasm32-unknown-unknown`. That allocator is intended for an isolated grammar
//! WebAssembly instance, but the browser build links it into the long-lived LSP
//! module. Using Rust's global allocator avoids its fixed 4 MiB arena.

use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc};
use std::ffi::c_void;
use std::ptr;
use std::sync::Once;

const ALIGN: usize = 16;
const HEADER_SIZE: usize = ALIGN;

static INSTALL: Once = Once::new();

pub(crate) fn install() {
    INSTALL.call_once(|| {
        // SAFETY: This runs before this crate creates any Tree-sitter object,
        // and the callbacks remain installed for the lifetime of the module.
        unsafe {
            tree_sitter::set_allocator(Some(malloc), Some(calloc), Some(reallocate), Some(free));
        }
    });
}

fn layout(payload_size: usize) -> Option<Layout> {
    let allocation_size = HEADER_SIZE.checked_add(payload_size)?;
    Layout::from_size_align(allocation_size, ALIGN).ok()
}

unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    if size == 0 {
        return ptr::null_mut();
    }

    let Some(layout) = layout(size) else {
        return ptr::null_mut();
    };

    // SAFETY: `layout` has non-zero size and valid power-of-two alignment.
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: The header occupies the first `HEADER_SIZE` bytes. The returned
    // payload remains aligned to `ALIGN` because the header has that size.
    unsafe {
        base.cast::<usize>().write(size);
        base.add(HEADER_SIZE).cast()
    }
}

unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(payload_size) = count.checked_mul(size) else {
        return ptr::null_mut();
    };
    if payload_size == 0 {
        return ptr::null_mut();
    }

    let Some(layout) = layout(payload_size) else {
        return ptr::null_mut();
    };

    // SAFETY: `layout` has non-zero size and valid power-of-two alignment.
    let base = unsafe { alloc_zeroed(layout) };
    if base.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: `base` owns the complete allocation described by `layout`.
    unsafe {
        base.cast::<usize>().write(payload_size);
        base.add(HEADER_SIZE).cast()
    }
}

unsafe extern "C" fn reallocate(pointer: *mut c_void, new_size: usize) -> *mut c_void {
    if pointer.is_null() {
        // SAFETY: Forwarding the C `realloc(NULL, size)` case to `malloc`.
        return unsafe { malloc(new_size) };
    }
    if new_size == 0 {
        // SAFETY: `pointer` satisfies the allocator callback contract.
        unsafe { free(pointer) };
        return ptr::null_mut();
    }

    // SAFETY: Every non-null pointer received by this callback was returned by
    // `malloc`, `calloc`, or an earlier call to `reallocate`.
    let base = unsafe { pointer.cast::<u8>().sub(HEADER_SIZE) };
    let old_size = unsafe { base.cast::<usize>().read() };
    let Some(old_layout) = layout(old_size) else {
        return ptr::null_mut();
    };
    let Some(new_layout) = layout(new_size) else {
        return ptr::null_mut();
    };

    // SAFETY: `base` was allocated with `old_layout`. Rust's allocator retains
    // the original allocation when this operation returns null.
    let new_base = unsafe { realloc(base, old_layout, new_layout.size()) };
    if new_base.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: `new_base` owns `new_layout`, including the metadata header.
    unsafe {
        new_base.cast::<usize>().write(new_size);
        new_base.add(HEADER_SIZE).cast()
    }
}

unsafe extern "C" fn free(pointer: *mut c_void) {
    if pointer.is_null() {
        return;
    }

    // SAFETY: Every non-null pointer received by this callback was returned by
    // one of the allocation callbacks above.
    let base = unsafe { pointer.cast::<u8>().sub(HEADER_SIZE) };
    let size = unsafe { base.cast::<usize>().read() };
    let Some(layout) = layout(size) else {
        return;
    };

    // SAFETY: `base` was allocated using this exact layout.
    unsafe { dealloc(base, layout) };
}
