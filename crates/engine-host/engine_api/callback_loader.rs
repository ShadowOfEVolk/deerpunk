//! Gamelib-registered loaders (`v1:resources:register`).

use deerpunk_engine_api::dynamic_load::DynamicLoad;
use deerpunk_engine_common::lightmap_atlas::LightmapAtlasLayers;
use deerpunk_engine_common::material::MaterialAsset;
use deerpunk_engine_common::resource::loader::{LoadedAsset, Loader};
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::wad::RgbaImage;

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
            let frag = if query.is_empty() {
                String::new()
            } else {
                format!("#{query}")
            };
            return Err(format!(
                "loader '{}' failed: {path}{frag} (see gamelib log for detail)",
                self.id
            ));
        }
        if self.type_name == "dynamic" {
            let wrapper = unsafe { Box::from_raw(ptr as *mut DynamicLoad) };
            let tag = super::resources::read_str(wrapper.type_tag_ptr, wrapper.type_tag_len);
            let payload = wrapper.data;
            let drop_fn = wrapper.drop_data;
            return unpack_dynamic_payload(&tag, payload, drop_fn, path);
        }
        if self.type_name == "mesh" {
            let mesh = unsafe { *Box::from_raw(ptr as *mut Mesh) };
            return Ok(LoadedAsset {
                data: Box::new(mesh),
                size: 0,
                type_name: "mesh",
            });
        }
        if self.type_name == "rgba-image" {
            let img = unsafe { *Box::from_raw(ptr as *mut RgbaImage) };
            return Ok(LoadedAsset {
                data: Box::new(img),
                size: 0,
                type_name: "rgba-image",
            });
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

fn unpack_dynamic_payload(
    tag: &str,
    payload: *mut std::ffi::c_void,
    drop_fn: DropFn,
    path: &str,
) -> Result<LoadedAsset, String> {
    if payload.is_null() {
        return Err(format!("dynamic loader empty payload: {path}"));
    }
    match tag {
        "mesh" => {
            let mesh = unsafe { *Box::from_raw(payload as *mut Mesh) };
            Ok(LoadedAsset {
                data: Box::new(mesh),
                size: 0,
                type_name: "mesh",
            })
        }
        "rgba-image" => {
            let img = unsafe { *Box::from_raw(payload as *mut RgbaImage) };
            Ok(LoadedAsset {
                data: Box::new(img),
                size: 0,
                type_name: "rgba-image",
            })
        }
        "material" => {
            let mat = unsafe { *Box::from_raw(payload as *mut MaterialAsset) };
            Ok(LoadedAsset {
                data: Box::new(mat),
                size: 0,
                type_name: "material",
            })
        }
        "lightmap-atlas" => {
            let lm = unsafe { *Box::from_raw(payload as *mut LightmapAtlasLayers) };
            Ok(LoadedAsset {
                data: Box::new(lm),
                size: 0,
                type_name: "lightmap-atlas",
            })
        }
        "bsp" => Ok(LoadedAsset {
            data: Box::new(ExternAsset {
                ptr: payload,
                drop_fn,
            }),
            size: 0,
            type_name: "bsp",
        }),
        other => Err(format!("dynamic loader unknown type '{other}' for {path}")),
    }
}
