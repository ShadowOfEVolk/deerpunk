use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Attribute, Expr, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, ReturnType, Token,
    Type,
};

struct NamespaceAttr {
    name: LitStr,
    host_field: Option<Ident>,
    on_host: bool,
    scope_flags: u32,
}

fn parse_scope_flags(expr: &Expr) -> syn::Result<u32> {
    let Expr::Array(arr) = expr else {
        return Err(syn::Error::new_spanned(
            expr,
            "scope must be an array, e.g. scope = [\"server\", \"client\"]",
        ));
    };
    if arr.elems.is_empty() {
        return Err(syn::Error::new_spanned(expr, "scope array must not be empty"));
    }
    let mut flags = 0u32;
    for elem in &arr.elems {
        let Expr::Lit(expr_lit) = elem else {
            return Err(syn::Error::new_spanned(
                elem,
                "scope entries must be string literals",
            ));
        };
        let syn::Lit::Str(s) = &expr_lit.lit else {
            return Err(syn::Error::new_spanned(&expr_lit.lit, "scope entries must be strings"));
        };
        match s.value().as_str() {
            "server" => flags |= 1 << 0,
            "client" => flags |= 1 << 1,
            other => {
                return Err(syn::Error::new_spanned(
                    s,
                    format!("unknown scope `{other}` (expected \"server\" or \"client\")"),
                ));
            }
        }
    }
    Ok(flags)
}

impl Parse for NamespaceAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name: Option<LitStr> = None;
        let mut host_field: Option<Ident> = None;
        let mut on_host = false;
        let mut scope_flags: Option<u32> = None;

        let nested: Punctuated<Meta, Token![,]> = input.parse_terminated(Meta::parse, Token![,])?;
        for meta in nested {
            match &meta {
                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    if let Expr::Lit(expr_lit) = &nv.value {
                        if let syn::Lit::Str(s) = &expr_lit.lit {
                            name = Some(s.clone());
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("host_field") => {
                    if let Expr::Lit(expr_lit) = &nv.value {
                        if let syn::Lit::Str(s) = &expr_lit.lit {
                            host_field = Some(Ident::new(&s.value(), s.span()));
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("scope") => {
                    scope_flags = Some(parse_scope_flags(&nv.value)?);
                }
                Meta::Path(p) if p.is_ident("on_host") => on_host = true,
                _ => {}
            }
        }

        if !on_host && host_field.is_none() {
            return Err(syn::Error::new(
                input.span(),
                "use `host_field = \"cmd\"` or `on_host` for `impl Host`",
            ));
        }

        Ok(NamespaceAttr {
            name: name.ok_or_else(|| syn::Error::new(input.span(), "missing `name = \"v1:…\"`"))?,
            host_field,
            on_host,
            scope_flags: scope_flags.ok_or_else(|| {
                syn::Error::new(
                    input.span(),
                    "missing `scope = [\"server\", \"client\"]` (or a subset)",
                )
            })?,
        })
    }
}

struct BindAttr {
    api_name: Option<String>,
}

fn attr_ends_with(attr: &Attribute, name: &str) -> bool {
    attr.path()
        .segments
        .last()
        .map(|s| s.ident == name)
        .unwrap_or(false)
}

fn parse_bind_attr(attrs: &[Attribute]) -> Option<BindAttr> {
    for attr in attrs {
        if !attr_ends_with(attr, "bind") {
            continue;
        }
        let mut api_name = None;
        let Ok(nested) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
            continue;
        };
        for meta in nested {
            if let Meta::NameValue(nv) = meta {
                if nv.path.is_ident("name") {
                    if let Expr::Lit(expr_lit) = nv.value {
                        if let syn::Lit::Str(s) = expr_lit.lit {
                            api_name = Some(s.value());
                        }
                    }
                }
            }
        }
        return Some(BindAttr { api_name });
    }
    None
}

fn strip_methoddb_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|a| !attr_ends_with(a, "bind") && !attr_ends_with(a, "namespace"))
        .cloned()
        .collect()
}

enum AbiType {
    Str,
    Bytes,
    U64Slice,
    MutF32,
    MutTraceLineResult,
    ConstVoidPtr,
    MutVoidPtr,
    MutU8Ptr,
    MutF32Ptr,
    MutU64Ptr,
    I16,
    U16,
    I32,
    U32,
    U64,
    U8,
    F32,
    Usize,
    Bool,
    UmHookFn,
    CmdCallback,
    WorldQueryCallback,
    ConstF32Ptr,
    ConstU64Ptr,
    ConstVoidPtrPtr,
    ConstColliderPartPtr,
    MutCapsuleCastHitOut,
    ConstResourceLoaderDescPtr,
    ConstComponentRegisterDescPtr,
    ConstSpawnPrefabOptsPtr,
    ScheduleSystemFn,
}

fn path_last_ident(path: &syn::Path) -> Option<&Ident> {
    path.segments.last().map(|s| &s.ident)
}

fn is_c_void(path: &syn::Path) -> bool {
    path.is_ident("c_void")
}

fn classify_ty(ty: &Type) -> syn::Result<AbiType> {
    match ty {
        Type::Reference(r) => match &*r.elem {
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("str") => {
                Ok(AbiType::Str)
            }
            Type::Slice(slice) => {
                if matches!(&*slice.elem, Type::Path(p) if p.path.is_ident("u8")) {
                    Ok(AbiType::Bytes)
                } else if matches!(&*slice.elem, Type::Path(p) if p.path.is_ident("u64")) {
                    Ok(AbiType::U64Slice)
                } else {
                    Err(syn::Error::new_spanned(ty, "unsupported slice (only &[u8] / &[u64])"))
                }
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("f32") => {
                if r.mutability.is_some() {
                    Ok(AbiType::MutF32)
                } else {
                    Err(syn::Error::new_spanned(ty, "f32 must be `&mut f32` for out params"))
                }
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if r.mutability.is_some()
                    && path_last_ident(path)
                    .map(|i| i == "PhysicsTraceResult")
                    .unwrap_or(false) =>
            {
                Ok(AbiType::MutTraceLineResult)
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if r.mutability.is_some()
                    && path_last_ident(path)
                        .map(|i| i == "CapsuleCastHitOut")
                        .unwrap_or(false) =>
            {
                Ok(AbiType::MutCapsuleCastHitOut)
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if path_last_ident(path)
                    .map(|i| i == "ResourceLoaderDesc")
                    .unwrap_or(false) =>
            {
                Ok(AbiType::ConstResourceLoaderDescPtr)
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if path_last_ident(path)
                    .map(|i| i == "ComponentRegisterDesc")
                    .unwrap_or(false) =>
            {
                Ok(AbiType::ConstComponentRegisterDescPtr)
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if r.mutability.is_none()
                    && path_last_ident(path)
                        .map(|i| i == "SpawnPrefabOpts")
                        .unwrap_or(false) =>
            {
                Ok(AbiType::ConstSpawnPrefabOptsPtr)
            }
            _ => Err(syn::Error::new_spanned(
                ty,
                "unsupported reference (use &str, &[u8], &[u64], &mut f32, or &mut PhysicsTraceResult)",
            )),
        },
        Type::Ptr(ptr) => match &*ptr.elem {
            Type::Path(syn::TypePath { qself: None, path, .. }) if is_c_void(path) => {
                if ptr.mutability.is_some() {
                    Ok(AbiType::MutVoidPtr)
                } else {
                    Ok(AbiType::ConstVoidPtr)
                }
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("u8") => {
                if ptr.mutability.is_some() {
                    Ok(AbiType::MutU8Ptr)
                } else {
                    Err(syn::Error::new_spanned(ty, "expected `*mut u8`"))
                }
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("f32") => {
                if ptr.mutability.is_some() {
                    Ok(AbiType::MutF32Ptr)
                } else {
                    Ok(AbiType::ConstF32Ptr)
                }
            }
            Type::Path(syn::TypePath { qself: None, path, .. })
                if ptr.mutability.is_none()
                    && path_last_ident(path)
                        .map(|i| i == "ColliderPart")
                        .unwrap_or(false) =>
            {
                Ok(AbiType::ConstColliderPartPtr)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("u64") => {
                if ptr.mutability.is_some() {
                    Ok(AbiType::MutU64Ptr)
                } else {
                    Ok(AbiType::ConstU64Ptr)
                }
            }
            Type::Ptr(inner2)
                if ptr.mutability.is_none() && inner2.mutability.is_some() =>
            {
                match &*inner2.elem {
                    Type::Path(syn::TypePath { qself: None, path, .. }) if is_c_void(path) => {
                        Ok(AbiType::ConstVoidPtrPtr)
                    }
                    _ => Err(syn::Error::new_spanned(ty, "unsupported raw pointer type")),
                }
            }
            _ => Err(syn::Error::new_spanned(ty, "unsupported raw pointer type")),
        },
        Type::Path(syn::TypePath { qself: None, path, .. }) => {
            if path.is_ident("i16") {
                Ok(AbiType::I16)
            } else if path.is_ident("u16") {
                Ok(AbiType::U16)
            } else if path.is_ident("i32") {
                Ok(AbiType::I32)
            } else if path.is_ident("u32") {
                Ok(AbiType::U32)
            } else if path.is_ident("u64") {
                Ok(AbiType::U64)
            } else if path.is_ident("u8") {
                Ok(AbiType::U8)
            } else if path.is_ident("usize") {
                Ok(AbiType::Usize)
            } else if path.is_ident("f32") {
                Ok(AbiType::F32)
            } else if path.is_ident("bool") {
                Ok(AbiType::Bool)
            } else if path_last_ident(path).map(|i| i == "UmHookFn").unwrap_or(false) {
                Ok(AbiType::UmHookFn)
            } else if path_last_ident(path).map(|i| i == "CmdCallback").unwrap_or(false) {
                Ok(AbiType::CmdCallback)
            } else if path_last_ident(path)
                .map(|i| i == "WorldQueryCallback")
                .unwrap_or(false)
            {
                Ok(AbiType::WorldQueryCallback)
            } else if path_last_ident(path)
                .map(|i| i == "ScheduleSystemFn")
                .unwrap_or(false)
            {
                Ok(AbiType::ScheduleSystemFn)
            } else {
                Err(syn::Error::new_spanned(ty, "unsupported type for methoddb bind"))
            }
        }
        _ => Err(syn::Error::new_spanned(ty, "unsupported type for methoddb bind")),
    }
}

enum ReturnMode {
    StatusI32,
    RawI32,
    RawU32,
    RawU64,
    RawUsize,
    RawMutVoid,
    RawBool,
    RawF32,
}

fn classify_return(output: &ReturnType) -> syn::Result<ReturnMode> {
    match output {
        ReturnType::Default => Ok(ReturnMode::StatusI32),
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(t) if t.elems.is_empty() => Ok(ReturnMode::StatusI32),
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("i32") => {
                Ok(ReturnMode::RawI32)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("u32") => {
                Ok(ReturnMode::RawU32)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("u64") => {
                Ok(ReturnMode::RawU64)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("usize") => {
                Ok(ReturnMode::RawUsize)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("bool") => {
                Ok(ReturnMode::RawBool)
            }
            Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("f32") => {
                Ok(ReturnMode::RawF32)
            }
            Type::Ptr(ptr) => match &*ptr.elem {
                Type::Path(syn::TypePath { qself: None, path, .. })
                    if ptr.mutability.is_some() && is_c_void(path) =>
                {
                    Ok(ReturnMode::RawMutVoid)
                }
                _ => Err(syn::Error::new_spanned(ty, "unsupported pointer return")),
            },
            _ => Err(syn::Error::new_spanned(
                ty,
                "methoddb: supported returns: `()`, `i32`, `u32`, `u64`, `usize`, `bool`, `f32`, `*mut c_void`",
            )),
        },
    }
}

fn read_str_expr(idx: usize) -> TokenStream {
    let ptr = format_ident!("__arg{}_ptr", idx);
    let len = format_ident!("__arg{}_len", idx);
    quote! {
        {
            if #ptr.is_null() || #len == 0 {
                String::new()
            } else {
                unsafe {
                    let bytes = std::slice::from_raw_parts(#ptr, #len);
                    String::from_utf8_lossy(bytes).into_owned()
                }
            }
        }
    }
}

fn generate_tramp(
    ns: &NamespaceAttr,
    bind: &BindAttr,
    method: &ImplItemFn,
) -> syn::Result<TokenStream> {
    let api_suffix = bind
        .api_name
        .clone()
        .unwrap_or_else(|| method.sig.ident.to_string());
    let full_name = format!("{}:{}", ns.name.value(), api_suffix);
    let tramp_ident = format_ident!(
        "__methoddb_tramp_{}",
        full_name.replace(':', "_").replace('.', "_")
    );
    let rust_method = &method.sig.ident;
    let scope_flags = ns.scope_flags;
    let receiver = method.sig.receiver().ok_or_else(|| {
        syn::Error::new_spanned(method, "methoddb bind requires a receiver (`&self` or `&mut self`)")
    })?;
    let self_mut = receiver.mutability.is_some();

    let mut ffi_params: Vec<TokenStream> = vec![quote! { engine: *mut u8 }];
    let mut rust_args: Vec<TokenStream> = Vec::new();
    let mut pre_call: Vec<TokenStream> = Vec::new();

    let mut arg_idx = 0usize;
    for input in method.sig.inputs.iter().skip(1) {
        let FnArg::Typed(pt) = input else {
            return Err(syn::Error::new_spanned(input, "expected typed argument"));
        };
        let abi = classify_ty(&pt.ty)?;
        match abi {
            AbiType::Str => {
                let ptr = format_ident!("__arg{}_ptr", arg_idx);
                let len = format_ident!("__arg{}_len", arg_idx);
                ffi_params.push(quote! { #ptr: *const u8 });
                ffi_params.push(quote! { #len: usize });
                let s = format_ident!("__s{}", arg_idx);
                let read = read_str_expr(arg_idx);
                pre_call.push(quote! { let #s = #read; });
                rust_args.push(quote! { &#s });
                arg_idx += 1;
            }
            AbiType::I32 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: i32 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::U32 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: u32 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::U64 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: u64 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::U8 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: u8 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::Usize => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: usize });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::F32 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: f32 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::Bool => {
                let n = format_ident!("__arg{}", arg_idx);
                let b = format_ident!("__b{}", arg_idx);
                ffi_params.push(quote! { #n: i32 });
                pre_call.push(quote! { let #b = #n != 0; });
                rust_args.push(quote! { #b });
                arg_idx += 1;
            }
            AbiType::I16 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: i16 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::U16 => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: u16 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutF32 => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let out_ref = format_ident!("__out{}", arg_idx);
                ffi_params.push(quote! { #ptr: *mut f32 });
                pre_call.push(quote! {
                    if #ptr.is_null() {
                        return -1;
                    }
                    let #out_ref = unsafe { &mut *#ptr };
                });
                rust_args.push(quote! { #out_ref });
                arg_idx += 1;
            }
            AbiType::Bytes => {
                let ptr = format_ident!("__arg{}_ptr", arg_idx);
                let len = format_ident!("__arg{}_len", arg_idx);
                let slice = format_ident!("__bytes{}", arg_idx);
                ffi_params.push(quote! { #ptr: *const u8 });
                ffi_params.push(quote! { #len: usize });
                pre_call.push(quote! {
                    let #slice: &[u8] = if #ptr.is_null() || #len == 0 {
                        &[]
                    } else {
                        unsafe { std::slice::from_raw_parts(#ptr, #len) }
                    };
                });
                rust_args.push(quote! { #slice });
                arg_idx += 1;
            }
            AbiType::U64Slice => {
                let ptr = format_ident!("__arg{}_ptr", arg_idx);
                let len = format_ident!("__arg{}_len", arg_idx);
                let slice = format_ident!("__u64s{}", arg_idx);
                ffi_params.push(quote! { #ptr: *const u64 });
                ffi_params.push(quote! { #len: usize });
                pre_call.push(quote! {
                    let #slice: &[u64] = if #ptr.is_null() || #len == 0 {
                        &[]
                    } else {
                        unsafe { std::slice::from_raw_parts(#ptr, #len) }
                    };
                });
                rust_args.push(quote! { #slice });
                arg_idx += 1;
            }
            AbiType::ConstVoidPtr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *const std::ffi::c_void });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutVoidPtr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *mut std::ffi::c_void });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutU8Ptr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *mut u8 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutF32Ptr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *mut f32 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutU64Ptr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *mut u64 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutTraceLineResult => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let out_ref = format_ident!("__trace_out{}", arg_idx);
                ffi_params.push(quote! { #ptr: *mut crate::physics_trace::PhysicsTraceResult });
                pre_call.push(quote! {
                    if #ptr.is_null() {
                        return -1;
                    }
                    let #out_ref = unsafe { &mut *#ptr };
                });
                rust_args.push(quote! { #out_ref });
                arg_idx += 1;
            }
            AbiType::UmHookFn => {
                let cb = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #cb: extern "C" fn(*const u8, usize, *const u8, usize) });
                rust_args.push(quote! { #cb });
                arg_idx += 1;
            }
            AbiType::CmdCallback => {
                let cb = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! {
                    #cb: extern "C" fn(i32, *const *const u8, *const usize, *mut std::ffi::c_void)
                });
                rust_args.push(quote! { #cb });
                arg_idx += 1;
            }
            AbiType::WorldQueryCallback => {
                let cb = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! {
                    #cb: extern "C" fn(u64, *const *const std::ffi::c_void, usize, *mut std::ffi::c_void)
                });
                rust_args.push(quote! { #cb });
                arg_idx += 1;
            }
            AbiType::ConstF32Ptr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *const f32 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::ConstU64Ptr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *const u64 });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::ConstVoidPtrPtr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! { #n: *const *mut std::ffi::c_void });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::ConstResourceLoaderDescPtr => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let desc_ref = format_ident!("__desc{}", arg_idx);
                ffi_params.push(quote! {
                    #ptr: *const deerpunk_engine_api::resources::ResourceLoaderDesc
                });
                pre_call.push(quote! {
                    if #ptr.is_null() {
                        return -1;
                    }
                    let #desc_ref = unsafe { &*#ptr };
                });
                rust_args.push(quote! { #desc_ref });
                arg_idx += 1;
            }
            AbiType::ConstComponentRegisterDescPtr => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let desc_ref = format_ident!("__desc{}", arg_idx);
                ffi_params.push(quote! {
                    #ptr: *const deerpunk_engine_api::components::ComponentRegisterDesc
                });
                pre_call.push(quote! {
                    if #ptr.is_null() {
                        return 0;
                    }
                    let #desc_ref = unsafe { &*#ptr };
                });
                rust_args.push(quote! { #desc_ref });
                arg_idx += 1;
            }
            AbiType::ConstSpawnPrefabOptsPtr => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let desc_ref = format_ident!("__spawn_opts{}", arg_idx);
                ffi_params.push(quote! {
                    #ptr: *const deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts
                });
                pre_call.push(quote! {
                    let #desc_ref = if #ptr.is_null() {
                        &deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts::empty()
                    } else {
                        unsafe { &*#ptr }
                    };
                });
                rust_args.push(quote! { #desc_ref });
                arg_idx += 1;
            }
            AbiType::ScheduleSystemFn => {
                let cb = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! {
                    #cb: extern "C" fn(f32, *mut std::ffi::c_void)
                });
                rust_args.push(quote! { #cb });
                arg_idx += 1;
            }
            AbiType::ConstColliderPartPtr => {
                let n = format_ident!("__arg{}", arg_idx);
                ffi_params.push(quote! {
                    #n: *const deerpunk_engine_common::physics::ColliderPart
                });
                rust_args.push(quote! { #n });
                arg_idx += 1;
            }
            AbiType::MutCapsuleCastHitOut => {
                let ptr = format_ident!("__arg{}", arg_idx);
                let out_ref = format_ident!("__caps_out{}", arg_idx);
                ffi_params.push(quote! {
                    #ptr: *mut crate::engine_api::physics::CapsuleCastHitOut
                });
                pre_call.push(quote! {
                    if #ptr.is_null() {
                        return -1;
                    }
                    let #out_ref = unsafe { &mut *#ptr };
                });
                rust_args.push(quote! { #out_ref });
                arg_idx += 1;
            }
        }
    }

    let host_ref = if self_mut {
        quote! { let host = unsafe { &mut *(engine as *mut crate::Host) }; }
    } else {
        quote! { let host = unsafe { &*(engine as *const crate::Host) }; }
    };

    let invoke = if ns.on_host {
        if self_mut {
            quote! { host.#rust_method(#(#rust_args,)*) }
        } else {
            quote! { host.#rust_method(#(#rust_args,)*) }
        }
    } else {
        let field = ns.host_field.as_ref().expect("host_field");
        if self_mut {
            quote! { host.#field.#rust_method(#(#rust_args,)*) }
        } else {
            quote! { host.#field.#rust_method(#(#rust_args,)*) }
        }
    };

    let return_mode = classify_return(&method.sig.output)?;
    let return_stmt = match return_mode {
        ReturnMode::StatusI32 => {
            if matches!(&method.sig.output, ReturnType::Type(_, ty) if {
                matches!(&**ty, Type::Path(p) if p.path.is_ident("i32"))
            }) {
                quote! { #invoke }
            } else {
                quote! {
                    #invoke;
                    0
                }
            }
        }
        ReturnMode::RawI32 => quote! { #invoke },
        ReturnMode::RawBool => quote! { #invoke },
        ReturnMode::RawF32 => quote! { #invoke },
        ReturnMode::RawU64 => quote! { #invoke },
        ReturnMode::RawU32 => quote! { #invoke },
        ReturnMode::RawUsize => quote! { #invoke },
        ReturnMode::RawMutVoid => quote! { #invoke },
    };

    let ffi_ret = match return_mode {
        ReturnMode::RawU64 => quote! { u64 },
        ReturnMode::RawU32 => quote! { u32 },
        ReturnMode::RawUsize => quote! { usize },
        ReturnMode::RawMutVoid => quote! { *mut std::ffi::c_void },
        ReturnMode::RawBool => quote! { bool },
        ReturnMode::RawF32 => quote! { f32 },
        _ => quote! { i32 },
    };

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub extern "C" fn #tramp_ident(#(#ffi_params,)*) -> #ffi_ret {
            #host_ref
            #(#pre_call)*
            #return_stmt
        }

        ::inventory::submit! {
            crate::engine_api::engine_api::EngineApiMethod {
                name: #full_name,
                callback: #tramp_ident as *const (),
                flags: #scope_flags,
            }
        }
    })
}

pub fn bind_attr(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

pub fn namespace_attr(attr: TokenStream, item: TokenStream) -> TokenStream {
    match namespace_impl(attr, item) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
}

fn namespace_impl(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let ns: NamespaceAttr = syn::parse2(attr)?;
    let mut impl_item: ItemImpl = syn::parse2(item)?;

    let mut tramps = Vec::new();

    for impl_item_inner in &mut impl_item.items {
        if let ImplItem::Fn(method) = impl_item_inner {
            let bind = parse_bind_attr(&method.attrs);
            if bind.is_none() {
                continue;
            }
            let bind = bind.unwrap();
            tramps.push(generate_tramp(&ns, &bind, method)?);
            method.attrs = strip_methoddb_attrs(&method.attrs);
        }
    }

    Ok(quote! {
        #impl_item
        #(#tramps)*
    })
}
