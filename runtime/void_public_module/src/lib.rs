//! An `EcsModule` implementation of `void_public`.
//!
//! This crate runs `build_tools` on `void_public` and generates the C FFI.

use std::{
    error::Error,
    ffi::{CStr, c_char, c_void},
    mem::MaybeUninit,
    slice,
};

use platform::{DeserializeReadFn, SerializeWriteFn};
use void_public::*;

/// A helper function to deserialize a resource using the C FFI.
pub fn resource_deserialize_ffi(
    resource_deserialize_c: unsafe extern "C" fn(
        string_id: *const c_char,
        val: *mut c_void,
        reader: *mut c_void,
        read: unsafe extern "C" fn(reader: *mut c_void, buf: *mut c_void, len: usize) -> isize,
    ) -> i32,
    string_id: &CStr,
    val: &mut [MaybeUninit<u8>],
    mut read: DeserializeReadFn<'_>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    unsafe extern "C" fn read_ffi(reader: *mut c_void, buf: *mut c_void, len: usize) -> isize {
        let buf = unsafe { slice::from_raw_parts_mut(buf.cast(), len) };
        let reader = unsafe { reader.cast::<DeserializeReadFn<'_>>().as_mut().unwrap() };

        match reader(buf) {
            Ok(len) => len as isize,
            Err(err) => {
                log::error!("{err}");
                -1
            }
        }
    }

    // We take a reference to a reference here, so that we can reference a
    // dynamic trait object (16 bits) via a pointer (8 bits).
    let read_ptr = &mut read as *mut DeserializeReadFn<'_>;

    let res = unsafe {
        resource_deserialize_c(
            string_id.as_ptr(),
            val.as_mut_ptr().cast(),
            read_ptr.cast(),
            read_ffi,
        )
    };

    if res == 0 {
        Ok(())
    } else {
        Err(format!("error code ({res})").into())
    }
}

/// A helper function to serialize a resource using the C FFI.
pub fn resource_serialize_ffi(
    resource_serialize_c: unsafe extern "C" fn(
        string_id: *const c_char,
        val: *const c_void,
        writer: *mut c_void,
        write: unsafe extern "C" fn(writer: *mut c_void, buf: *const c_void, len: usize) -> isize,
    ) -> i32,
    string_id: &CStr,
    val: &[MaybeUninit<u8>],
    mut write: SerializeWriteFn<'_>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    unsafe extern "C" fn write_ffi(writer: *mut c_void, buf: *const c_void, len: usize) -> isize {
        let buf = unsafe { slice::from_raw_parts(buf.cast(), len) };
        let writer = unsafe { writer.cast::<SerializeWriteFn<'_>>().as_mut().unwrap() };

        match writer(buf) {
            Ok(len) => len as isize,
            Err(err) => {
                log::error!("{err}");
                -1
            }
        }
    }

    // We take a reference to a reference here, so that we can reference a
    // dynamic trait object (16 bits) via a pointer (8 bits).
    let write_ptr = &mut write as *mut SerializeWriteFn<'_>;

    let res = unsafe {
        resource_serialize_c(
            string_id.as_ptr(),
            val.as_ptr().cast(),
            write_ptr.cast(),
            write_ffi,
        )
    };

    if res == 0 {
        Ok(())
    } else {
        Err(format!("error code ({res})").into())
    }
}

pub fn component_deserialize_json_ffi(
    component_deserialize_json_c: unsafe extern "C" fn(
        string_id: *const c_char,
        val: *mut c_void,
        json_text: *const c_void,
        json_text_len: usize,
    ) -> i32,
    string_id: &CStr,
    val: &mut [MaybeUninit<u8>],
    json_string: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let res = unsafe {
        // get a pointer to the size of json string and send as pointer and length
        let json_bytes = json_string.as_bytes();
        component_deserialize_json_c(
            string_id.as_ptr(),
            val.as_mut_ptr().cast(),
            json_bytes.as_ptr().cast(),
            json_bytes.len(),
        )
    };
    if res == 0 {
        Ok(())
    } else {
        Err(format!("error code ({res})").into())
    }
}

pub mod ffi {
    use super::*;

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
}
