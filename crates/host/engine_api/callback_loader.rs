//! Gamelib-registered loaders (`v1:resources:register`).

use deerpunk_engine_common::resource::loader::{LoadedAsset, Loader};

pub type LoadFn = extern "C" fn(
    user: *mut std::ffi::c_void,
    path_ptr: *const u8,
    path_len: usize,
    query_ptr: *const u8,
    query_len: usize,
) -> *mut std::ffi::c_void;

pub type DropFn = extern "C" fn(*mut std::ffi::c_void);

struct ExternAsset {
    ptr: *mut std::ffi::c_void,
    drop_fn: DropFn,
}

// Opaque gamelib pointer; gamelib owns thread-safety of its loader user_data.
unsafe impl Send for ExternAsset {}
unsafe impl Sync for ExternAsset {}

impl Drop for ExternAsset {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            (self.drop_fn)(self.ptr);
        }
    }
}

pub struct CallbackLoader {
    id: String,
    extensions: &'static [&'static str],
    type_name: &'static str,
    user: *mut std::ffi::c_void,
    load_fn: LoadFn,
    drop_fn: DropFn,
}

// user_data is owned by gamelib and only touched from host load path.
unsafe impl Send for CallbackLoader {}
unsafe impl Sync for CallbackLoader {}

impl CallbackLoader {
    pub fn new(
        id: String,
        extensions: Vec<String>,
        type_name: String,
        user: *mut std::ffi::c_void,
        load_fn: LoadFn,
        drop_fn: DropFn,
    ) -> Self {
        let ext_vec: Vec<&'static str> = extensions
            .into_iter()
            .map(|e| Box::leak(e.into_boxed_str()) as &'static str)
            .collect();
        let extensions = Box::leak(ext_vec.into_boxed_slice());
        let type_name: &'static str = Box::leak(type_name.into_boxed_str());
        Self {
            id,
            extensions,
            type_name,
            user,
            load_fn,
            drop_fn,
        }
    }
}

impl Loader for CallbackLoader {
    fn id(&self) -> &str {
        &self.id
    }

    fn extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    fn load(&self, path: &str, query: &str) -> Result<LoadedAsset, String> {
        let ptr = (self.load_fn)(
            self.user,
            path.as_ptr(),
            path.len(),
            query.as_ptr(),
            query.len(),
        );
        if ptr.is_null() {
            return Err(format!("loader '{}' failed: {path}", self.id));
        }
        Ok(LoadedAsset {
            data: Box::new(ExternAsset {
                ptr,
                drop_fn: self.drop_fn,
            }),
            size: 0,
            type_name: self.type_name,
        })
    }
}
