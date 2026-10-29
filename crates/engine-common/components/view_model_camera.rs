//! Marks a dedicated first-person viewmodel camera (child of the view camera).

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewModelCameraTag {
    pub _tag: u8,
}

impl ViewModelCameraTag {
    pub const fn marker() -> Self {
        Self { _tag: 0 }
    }
}
