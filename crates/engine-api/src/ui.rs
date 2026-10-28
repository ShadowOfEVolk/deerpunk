use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type StrFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type DocFieldBoolFn =
    extern "C" fn(*mut u8, *const u8, usize, *const u8, usize, i32) -> i32;
type DocFieldF32Fn =
    extern "C" fn(*mut u8, *const u8, usize, *const u8, usize, f32) -> i32;
type DocFieldStrFn = extern "C" fn(
    *mut u8,
    *const u8,
    usize,
    *const u8,
    usize,
    *const u8,
    usize,
) -> i32;

/// Value for [`UiApi::data_model_set`] (RmlUi `data-model` field).
#[derive(Clone, Debug)]
pub enum DataModelValue {
    Bool(bool),
    Number(f64),
    String(String),
}

/// Client UI widget list API (`v1:ui:*`).
pub struct UiApi {
    ctx: *mut u8,
    load_fn: StrFn,
    unload_fn: StrFn,
    show_fn: StrFn,
    hide_fn: StrFn,
    lua_eval_fn: StrFn,
    data_model_set_bool_fn: DocFieldBoolFn,
    data_model_set_number_fn: DocFieldF32Fn,
    data_model_set_string_fn: DocFieldStrFn,
}

unsafe impl Send for UiApi {}
unsafe impl Sync for UiApi {}

impl UiApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            load_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:load")) },
            unload_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:unload")) },
            show_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:show")) },
            hide_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:hide")) },
            lua_eval_fn: unsafe { std::mem::transmute(resolve(r, "v1:ui:lua_eval")) },
            data_model_set_bool_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:ui:data_model_set_bool"))
            },
            data_model_set_number_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:ui:data_model_set_number"))
            },
            data_model_set_string_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:ui:data_model_set_string"))
            },
        }
    }

    fn doc_field_call_bool(f: DocFieldBoolFn, ctx: *mut u8, doc: &str, field: &str, v: bool) -> Result<(), String> {
        let doc_b = doc.as_bytes();
        let field_b = field.as_bytes();
        let rc = f(
            ctx,
            doc_b.as_ptr(),
            doc_b.len(),
            field_b.as_ptr(),
            field_b.len(),
            v as i32,
        );
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("data_model_set_bool failed: {doc}.{field}"))
        }
    }

    fn doc_field_call_f32(f: DocFieldF32Fn, ctx: *mut u8, doc: &str, field: &str, v: f32) -> Result<(), String> {
        let doc_b = doc.as_bytes();
        let field_b = field.as_bytes();
        let rc = f(
            ctx,
            doc_b.as_ptr(),
            doc_b.len(),
            field_b.as_ptr(),
            field_b.len(),
            v,
        );
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("data_model_set_number failed: {doc}.{field}"))
        }
    }

    fn doc_field_call_str(
        f: DocFieldStrFn,
        ctx: *mut u8,
        doc: &str,
        field: &str,
        value: &str,
    ) -> Result<(), String> {
        let doc_b = doc.as_bytes();
        let field_b = field.as_bytes();
        let val_b = value.as_bytes();
        let rc = f(
            ctx,
            doc_b.as_ptr(),
            doc_b.len(),
            field_b.as_ptr(),
            field_b.len(),
            val_b.as_ptr(),
            val_b.len(),
        );
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("data_model_set_string failed: {doc}.{field}"))
        }
    }

    fn call(f: StrFn, ctx: *mut u8, id: &str) -> Result<(), String> {
        let bytes = id.as_bytes();
        let rc = f(ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("ui op failed: {id}"))
        }
    }

    /// Load document into the widget list (hidden). Id: `main_menu` or `widgets/main_menu.rml`.
    pub fn load(&self, id: &str) -> Result<(), String> {
        Self::call(self.load_fn, self.ctx, id)
    }

    pub fn unload(&self, id: &str) -> Result<(), String> {
        Self::call(self.unload_fn, self.ctx, id)
    }

    pub fn show(&self, id: &str) -> Result<(), String> {
        Self::call(self.show_fn, self.ctx, id)
    }

    pub fn hide(&self, id: &str) -> Result<(), String> {
        Self::call(self.hide_fn, self.ctx, id)
    }

    /// Execute Lua in the RmlUi interpreter (content-owned HUD / menus).
    pub fn lua_eval(&self, code: &str) -> Result<(), String> {
        Self::call(self.lua_eval_fn, self.ctx, code)
    }

    /// Set a field on the widget document's `data-model` (`data-bind` / `data-class-*` names).
    pub fn data_model_set(
        &self,
        document: &str,
        field: &str,
        value: DataModelValue,
    ) -> Result<(), String> {
        match value {
            DataModelValue::Bool(v) => Self::doc_field_call_bool(
                self.data_model_set_bool_fn,
                self.ctx,
                document,
                field,
                v,
            ),
            DataModelValue::Number(v) => Self::doc_field_call_f32(
                self.data_model_set_number_fn,
                self.ctx,
                document,
                field,
                v as f32,
            ),
            DataModelValue::String(v) => Self::doc_field_call_str(
                self.data_model_set_string_fn,
                self.ctx,
                document,
                field,
                &v,
            ),
        }
    }
}
