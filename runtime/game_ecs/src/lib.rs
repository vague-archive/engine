//! Core functionality of the ECS execution.
//!
//! This allows for construction of a graph of systems and executing those
//! systems with a safe interface wrapper around resources used by those
//! systems, for both CPU and GPU systems.

use std::mem::MaybeUninit;

pub use crate::{
    archetype::{ArchetypeKey, ArchetypeStorage, ArchetypeStorageMap},
    callables::Callables,
    component::{
        AsyncCompletionInfo, CallableInfo, ComponentBundle, ComponentDefault, ComponentInfo,
        ComponentRegistry, EcsTypeInfo, EntityComponentInfo, ResourceInfo,
        manually_register_resource,
    },
    cpu_frame_data::{CpuDataBuffer, CpuFrameData},
    system::{
        EcsSystem, EcsSystemExecuteResources, SystemGraph, WorldDelegate, add_components_helper,
        bundle_required_components, system_execute_resources,
    },
};

mod archetype;
mod callables;
mod component;
pub mod cpu_frame_data;
mod system;

pub trait GpuFrameData: std::fmt::Debug + Send + Sync + 'static {
    type FrameDataBufferBorrowRef: FrameDataBufferBorrowRef;

    type FrameDataBufferBorrowRefMut: FrameDataBufferBorrowRefMut;

    type FrameDataBufferRef<'a>: FrameDataBufferRef<'a>
    where
        Self: 'a;

    type FrameDataBufferRefMut<'a>: FrameDataBufferRefMut<'a>
    where
        Self: 'a;

    fn new_buffer(&mut self, cpu_data: &mut CpuFrameData, stride: usize) -> usize;

    fn allocate_buffer_partition(&mut self, index: usize) -> PartitionIndex;

    fn buffers_len(&self) -> usize;

    fn buffer_total_len(&self, index: usize) -> usize;

    fn borrow_buffer(
        &self,
        index: usize,
        partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRef;

    /// Returns the previous frame's buffer. Panics if `GpuFrameData::MULTI_BUFFERED == false`.
    fn borrow_buffer_prev(
        &self,
        index: usize,
        partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRef;

    fn borrow_buffer_mut(
        &self,
        index: usize,
        partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRefMut;

    fn get_buffer_mut(
        &mut self,
        cpu_data: &mut CpuFrameData,
        index: usize,
        partition: PartitionIndex,
    ) -> Self::FrameDataBufferRefMut<'_>;

    /// Returns `frames_behind` buffer. Panics if `frames_behind >= MULTI_BUFFER_LEN`.
    fn get_buffer_prev(
        &mut self,
        index: usize,
        partition: PartitionIndex,
        frames_behind: usize,
    ) -> Self::FrameDataBufferRef<'_>;
}

pub type PartitionIndex = u16;

pub trait FrameDataBufferBorrowRef: std::fmt::Debug + Send + Sync {
    /// # Safety
    ///
    /// Caller must guarantee that entry T is valid and initialized.
    unsafe fn get_as<T>(&self, index: usize) -> Option<&T>;

    /// Returns type T with `offset` bytes within the strided index.
    ///
    /// # Safety
    ///
    /// Caller must guarantee that entry T is valid and initialized, and that the offset is correct.
    unsafe fn get_with_offset_as<T>(&self, index: usize, offset: usize) -> Option<&T>;

    // Buffer properties

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn has_been_copied_this_frame(&self) -> bool;

    // Iteration access

    fn get_ptr(&self, index: usize) -> *const MaybeUninit<u8>;
}

pub trait FrameDataBufferBorrowRefMut: FrameDataBufferBorrowRef {
    /// # Safety
    ///
    /// Caller must guarantee that entry T is valid and initialized.
    unsafe fn get_mut_as<T>(&mut self, index: usize) -> Option<&mut T>;

    /// Returns type T with `offset` bytes within the strided index.
    ///
    /// # Safety
    ///
    /// Caller must guarantee that entry T is valid and initialized, and that the offset is correct.
    unsafe fn get_mut_with_offset_as<T>(&mut self, index: usize, offset: usize) -> Option<&mut T>;

    // Buffer properties

    fn mark_has_been_copied_this_frame(&mut self);

    // Iteration access

    fn get_mut_ptr(&mut self, index: usize) -> *mut MaybeUninit<u8>;
}

pub trait FrameDataBufferRef<'a>: std::fmt::Debug + FrameDataBufferBorrowRef {}

pub trait FrameDataBufferRefMut<'a>: FrameDataBufferBorrowRefMut {
    fn write<T: 'static>(&mut self, index: usize, val: T);

    fn push<T: 'static>(&mut self, val: T);

    fn grow(&mut self) -> &mut [MaybeUninit<u8>];

    /// # Safety
    ///
    /// Caller must guarantee that entry T is valid and initialized.
    unsafe fn pop<T>(&mut self) -> Option<T>;

    fn swap_remove(&mut self, index: usize);
}
