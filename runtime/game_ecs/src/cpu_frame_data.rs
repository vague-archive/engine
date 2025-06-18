use std::{
    mem::{ManuallyDrop, MaybeUninit, align_of, size_of},
    ptr,
};

use aligned_vec::AVec;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use void_public::Resource;

use crate::{
    ComponentRegistry, EcsTypeInfo, FrameDataBufferBorrowRef, FrameDataBufferBorrowRefMut,
    FrameDataBufferRefMut,
};

#[cfg(feature = "state_snapshots")]
mod serialize;

#[derive(Default)]
pub struct CpuFrameData {
    buffers: Vec<AtomicRefCell<CpuDataBuffer>>,
}

impl CpuFrameData {
    pub fn new_buffer(&mut self, stride: usize, align: usize) -> usize {
        let len = self.buffers_len();
        self.buffers.push(CpuDataBuffer::new(stride, align).into());
        len
    }

    pub fn buffers_len(&self) -> usize {
        self.buffers.len()
    }

    #[track_caller]
    pub fn borrow_buffer(&self, index: usize) -> AtomicRef<'_, CpuDataBuffer> {
        self.buffers[index].borrow()
    }

    pub fn borrow_buffer_prev(&self, _index: usize) -> Option<AtomicRef<'_, CpuDataBuffer>> {
        None
    }

    #[track_caller]
    pub fn borrow_buffer_mut(&self, index: usize) -> AtomicRefMut<'_, CpuDataBuffer> {
        self.buffers[index].borrow_mut()
    }

    #[track_caller]
    pub fn get_buffer_mut(&mut self, index: usize) -> CpuDataBufferRefMut<'_> {
        CpuDataBufferRefMut(self.buffers[index].get_mut())
    }

    /// Returns a pair of mutable buffer references.
    ///
    /// This function serves a similar purpose as `split_mut_at()`, in that it
    /// safely returns two mutable entries from a single container.
    ///
    /// # Panics
    ///
    /// Panics if the indices in the tuple are equal.
    #[track_caller]
    pub fn get_buffer_pair_mut(
        &mut self,
        indices: (usize, usize),
    ) -> (CpuDataBufferRefMut<'_>, CpuDataBufferRefMut<'_>) {
        assert_ne!(indices.0, indices.1, "indices must be unique");

        if indices.0 < indices.1 {
            let (a, b) = self.buffers.split_at_mut(indices.1);
            (
                CpuDataBufferRefMut(a[indices.0].get_mut()),
                CpuDataBufferRefMut(b[0].get_mut()),
            )
        } else {
            let (a, b) = self.buffers.split_at_mut(indices.0);
            (
                CpuDataBufferRefMut(b[0].get_mut()),
                CpuDataBufferRefMut(a[indices.1].get_mut()),
            )
        }
    }

    #[track_caller]
    pub fn get_buffer_prev(
        &mut self,
        _index: usize,
        _frames_behind: usize,
    ) -> Option<&CpuDataBuffer> {
        None
    }

    /// Gets an immutable reference to a registered Resource. This takes
    /// `&mut self` to bypass threadsafety checks.
    pub fn get_resource<R, F, T>(&mut self, component_registry: &ComponentRegistry, f: F) -> T
    where
        R: Resource,
        F: FnOnce(&R) -> T,
    {
        let buffer_index = match &component_registry[&R::id()].ecs_type_info {
            EcsTypeInfo::Resource(info) => info.buffer_index,
            _ => unreachable!(),
        };

        let buffer = self.get_buffer_mut(buffer_index);
        let resource = unsafe { buffer.get_as(0).unwrap() };
        f(resource)
    }

    /// Gets a mutable reference to a registered Resource.
    pub fn get_resource_mut<R, F, T>(&mut self, component_registry: &ComponentRegistry, f: F) -> T
    where
        R: Resource,
        F: FnOnce(&mut R) -> T,
    {
        let buffer_index = match &component_registry[&R::id()].ecs_type_info {
            EcsTypeInfo::Resource(info) => info.buffer_index,
            _ => unreachable!(),
        };

        let mut buffer = self.get_buffer_mut(buffer_index);
        let resource = unsafe { buffer.get_mut_as(0).unwrap() };
        f(resource)
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct CpuDataBuffer {
    data: AVec<MaybeUninit<u8>>,
    stride: usize,
    len: usize,
}

impl CpuDataBuffer {
    fn new(stride: usize, align: usize) -> Self {
        Self {
            data: AVec::new(align),
            stride,
            len: 0,
        }
    }
}

impl FrameDataBufferBorrowRef for CpuDataBuffer {
    unsafe fn get_as<T>(&self, index: usize) -> Option<&T> {
        if index < self.len {
            let offset = self.stride * index;
            let ptr = unsafe { self.data.as_ptr().add(offset).cast::<T>() };

            assert_eq!(ptr as usize % align_of::<T>(), 0);

            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    unsafe fn get_with_offset_as<T>(&self, index: usize, offset: usize) -> Option<&T> {
        if index < self.len {
            let offset = self.stride * index + offset;
            let ptr = unsafe { self.data.as_ptr().add(offset).cast::<T>() };

            assert_eq!(ptr as usize % align_of::<T>(), 0);

            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn has_been_copied_this_frame(&self) -> bool {
        panic!("CPU buffers are not multi-buffered");
    }

    #[inline]
    #[track_caller]
    fn get_ptr(&self, index: usize) -> *const MaybeUninit<u8> {
        assert!(index < self.len);
        unsafe { self.data.as_ptr().add(self.stride * index) }
    }
}

impl FrameDataBufferBorrowRefMut for CpuDataBuffer {
    unsafe fn get_mut_as<T>(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            let offset = self.stride * index;
            let ptr = unsafe { self.data.as_mut_ptr().add(offset).cast::<T>() };

            assert_eq!(ptr as usize % align_of::<T>(), 0);

            Some(unsafe { &mut *ptr })
        } else {
            None
        }
    }

    unsafe fn get_mut_with_offset_as<T>(&mut self, index: usize, offset: usize) -> Option<&mut T> {
        if index < self.len {
            let offset = self.stride * index + offset;
            let ptr = unsafe { self.data.as_mut_ptr().add(offset).cast::<T>() };

            assert_eq!(ptr as usize % align_of::<T>(), 0);

            Some(unsafe { &mut *ptr })
        } else {
            None
        }
    }

    fn mark_has_been_copied_this_frame(&mut self) {
        panic!("CPU buffers are not multi-buffered");
    }

    #[inline]
    #[track_caller]
    fn get_mut_ptr(&mut self, index: usize) -> *mut MaybeUninit<u8> {
        assert!(index < self.len);
        unsafe { self.data.as_mut_ptr().add(self.stride * index) }
    }
}

#[derive(Debug)]
pub struct CpuDataBufferRefMut<'a>(&'a mut CpuDataBuffer);

impl FrameDataBufferBorrowRef for CpuDataBufferRefMut<'_> {
    unsafe fn get_as<T>(&self, index: usize) -> Option<&T> {
        unsafe { self.0.get_as(index) }
    }

    unsafe fn get_with_offset_as<T>(&self, index: usize, offset: usize) -> Option<&T> {
        unsafe { self.0.get_with_offset_as(index, offset) }
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn has_been_copied_this_frame(&self) -> bool {
        self.0.has_been_copied_this_frame()
    }

    #[inline]
    #[track_caller]
    fn get_ptr(&self, index: usize) -> *const MaybeUninit<u8> {
        self.0.get_ptr(index)
    }
}

impl FrameDataBufferBorrowRefMut for CpuDataBufferRefMut<'_> {
    unsafe fn get_mut_as<T>(&mut self, index: usize) -> Option<&mut T> {
        unsafe { self.0.get_mut_as(index) }
    }

    unsafe fn get_mut_with_offset_as<T>(&mut self, index: usize, offset: usize) -> Option<&mut T> {
        unsafe { self.0.get_mut_with_offset_as(index, offset) }
    }

    fn mark_has_been_copied_this_frame(&mut self) {
        self.0.mark_has_been_copied_this_frame();
    }

    #[inline]
    #[track_caller]
    fn get_mut_ptr(&mut self, index: usize) -> *mut MaybeUninit<u8> {
        self.0.get_mut_ptr(index)
    }
}

impl<'a> FrameDataBufferRefMut<'a> for CpuDataBufferRefMut<'a> {
    fn write<T>(&mut self, index: usize, val: T) {
        let val = ManuallyDrop::new(val);

        let offset = self.0.stride * index;
        let slice = &mut self.0.data[offset..offset + size_of::<T>()];

        assert_eq!(slice.as_ptr() as usize % align_of::<T>(), 0);

        unsafe {
            ptr::copy_nonoverlapping(
                (&val as *const ManuallyDrop<T>).cast(),
                slice.as_mut_ptr(),
                size_of::<T>(),
            );
        }
    }

    fn push<T>(&mut self, val: T) {
        let val = ManuallyDrop::new(val);

        let offset = self.0.stride * self.0.len;

        self.0.len += 1;
        self.0
            .data
            .resize(self.0.len * self.0.stride, MaybeUninit::uninit());

        let slice = &mut self.0.data[offset..offset + size_of::<T>()];

        assert_eq!(slice.as_ptr() as usize % align_of::<T>(), 0);

        unsafe {
            ptr::copy_nonoverlapping(
                (&val as *const ManuallyDrop<T>).cast(),
                slice.as_mut_ptr(),
                size_of::<T>(),
            );
        }
    }

    fn grow(&mut self) -> &mut [MaybeUninit<u8>] {
        let offset = self.0.stride * self.0.len;

        self.0.len += 1;
        self.0
            .data
            .resize(self.0.len * self.0.stride, MaybeUninit::uninit());

        &mut self.0.data[offset..offset + self.0.stride]
    }

    unsafe fn pop<T>(&mut self) -> Option<T> {
        if self.0.len == 0 {
            None
        } else {
            unsafe {
                self.0.len -= 1;
                let offset = self.0.stride * self.0.len;

                self.0.data.set_len(offset);

                let ptr = self.0.data.as_ptr().add(offset).cast();

                assert_eq!(ptr as usize % align_of::<T>(), 0);

                Some(ManuallyDrop::into_inner(ptr::read(ptr)))
            }
        }
    }

    fn swap_remove(&mut self, index: usize) {
        #[cold]
        #[inline(never)]
        fn assert_failed(index: usize, len: usize) -> ! {
            panic!("swap_remove index (is {index}) should be < len (is {len})");
        }

        if index >= self.0.len {
            assert_failed(index, self.0.len);
        }

        self.0.len -= 1;
        let byte_len = self.0.len * self.0.stride;

        if index < self.0.len {
            unsafe {
                let offset = self.0.stride * index;
                let src = self.0.data.as_ptr().add(byte_len);
                let dst = self.0.data.as_mut_ptr().add(offset);
                ptr::copy(src, dst, self.0.stride);
            }
        }

        self.0.data.truncate(byte_len);
    }
}
