//! Gamelib → host payload for loaders that return multiple asset types.

use std::ffi::c_void;

pub type DropFn = extern "C" fn(*mut c_void);

/// Box returned from a multiplex loader (`type_tag` = `"dynamic"`).
#[repr(C)]
pub struct DynamicLoad {
    pub type_tag_ptr: *const u8,
    pub type_tag_len: usize,
    pub data: *mut c_void,
    pub drop_data: DropFn,
}

/// Build a [`DynamicLoad`] box for the host callback loader.
pub fn pack(tag: &str, data: *mut c_void, drop_data: DropFn) -> *mut c_void {
    let boxed = Box::new(DynamicLoad {
        type_tag_ptr: tag.as_ptr(),
        type_tag_len: tag.len(),
        data,
        drop_data,
    });
    Box::into_raw(boxed) as *mut c_void
}

/// Drop the wrapper and the inner payload.
pub extern "C" fn drop_dynamic(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { Box::from_raw(ptr as *mut DynamicLoad) };
    if !wrapper.data.is_null() {
        (wrapper.drop_data)(wrapper.data);
    }
}
