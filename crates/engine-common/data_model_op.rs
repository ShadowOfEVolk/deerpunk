//! C layout for one `v1:ui:data_model_apply` batch op.
//!
//! Mirrors `rmlui_data_model_op_t` in `3rdparty/rmlui-sys/api.h`; see that
//! header for path and resize semantics.

/// `DataModelOp::tag`: write a bool.
pub const DATA_MODEL_OP_BOOL: u8 = 0;
/// `DataModelOp::tag`: write an f64 number.
pub const DATA_MODEL_OP_NUMBER: u8 = 1;
/// `DataModelOp::tag`: write a NUL-terminated UTF-8 string.
pub const DATA_MODEL_OP_STRING: u8 = 2;
/// `DataModelOp::tag`: set the length of the Lua array at the op path.
pub const DATA_MODEL_OP_RESIZE: u8 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub union DataModelOpValue {
    pub b: i32,
    pub num: f64,
    pub string: *const std::ffi::c_char,
    pub len: i64,
}

/// One batched data-model write. Strings are NUL-terminated and only read for
/// the duration of the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DataModelOp {
    pub path: *const std::ffi::c_char,
    pub tag: u8,
    pub _pad: [u8; 7],
    pub value: DataModelOpValue,
}
