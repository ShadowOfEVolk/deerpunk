use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Attribute, Expr, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, Token,
};

struct NamespaceAttr {
    name: LitStr,
    host_field: Ident,
    /// `EngineApiMethod::flags` (`deerpunk_engine_api::methoddb::SCOPE_*`).
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
        let lit = match elem {
            Expr::Lit(expr_lit) => &expr_lit.lit,
            _ => {
                return Err(syn::Error::new_spanned(
                    elem,
                    "scope entries must be string literals",
                ));
            }
        };
        let syn::Lit::Str(s) = lit else {
            return Err(syn::Error::new_spanned(lit, "scope entries must be strings"));
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
                _ => {}
            }
        }

        Ok(NamespaceAttr {
            name: name.ok_or_else(|| syn::Error::new(input.span(), "missing `name = \"v1:…\"`"))?,
            host_field: host_field
                .ok_or_else(|| syn::Error::new(input.span(), "missing `host_field = \"cmd\"`"))?,
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

fn ty_is_str(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => match &*r.elem {
            syn::Type::Path(syn::TypePath { qself: None, path, .. }) => path.is_ident("str"),
            _ => false,
        },
        _ => false,
    }
}

fn method_ident_str(method: &ImplItemFn) -> String {
    method.sig.ident.to_string()
}

fn generate_execute_tramp(
    ns: &NamespaceAttr,
    bind: &BindAttr,
    method: &ImplItemFn,
) -> syn::Result<TokenStream> {
    let api_suffix = bind
        .api_name
        .clone()
        .unwrap_or_else(|| method_ident_str(method));
    let full_name = format!("{}:{}", ns.name.value(), api_suffix);
    let tramp_ident = format_ident!(
        "__methoddb_tramp_{}",
        full_name.replace(':', "_").replace('.', "_")
    );

    let host_field = &ns.host_field;
    let rust_method = &method.sig.ident;
    let scope_flags = ns.scope_flags;

    let sig = &method.sig;
    if sig.inputs.len() != 2 {
        return Err(syn::Error::new_spanned(
            method,
            "methoddb: only `&self` + one argument supported for now",
        ));
    }
    let has_self = matches!(&sig.receiver(), Some(r) if r.mutability.is_none());
    if !has_self {
        return Err(syn::Error::new_spanned(
            method,
            "methoddb: only `&self` methods supported for now",
        ));
    }

    let arg = sig
        .inputs
        .iter()
        .skip(1)
        .next()
        .and_then(|a| match a {
            syn::FnArg::Typed(pt) => Some(pt),
            _ => None,
        })
        .ok_or_else(|| syn::Error::new_spanned(method, "methoddb: missing argument"))?;

    if !ty_is_str(&arg.ty) {
        return Err(syn::Error::new_spanned(
            &arg.ty,
            "methoddb: only `&str` argument is supported (C ABI uses ptr + len, not Rust `&str`)",
        ));
    }

    let returns_unit = match &sig.output {
        syn::ReturnType::Default => true,
        syn::ReturnType::Type(_, ty) => {
            matches!(&**ty, syn::Type::Tuple(t) if t.elems.is_empty())
        }
    };
    if !returns_unit {
        return Err(syn::Error::new_spanned(
            &sig.output,
            "methoddb: only `()` return for tramps (tramp returns `i32` status)",
        ));
    }

    let generated = quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub extern "C" fn #tramp_ident(
            engine: *mut u8,
            __arg0_ptr: *const u8,
            __arg0_len: usize,
        ) -> i32 {
            let __line = {
                if __arg0_ptr.is_null() || __arg0_len == 0 {
                    String::new()
                } else {
                    unsafe {
                        let bytes = std::slice::from_raw_parts(__arg0_ptr, __arg0_len);
                        String::from_utf8_lossy(bytes).into_owned()
                    }
                }
            };
            if __line.trim().is_empty() {
                return -1;
            }
            let host = unsafe { &*(engine as *const crate::Host) };
            host.#host_field.#rust_method(&__line);
            0
        }

        ::inventory::submit! {
            crate::engine_api::engine_api::EngineApiMethod {
                name: #full_name,
                callback: #tramp_ident as *const (),
                flags: #scope_flags,
            }
        }
    };

    Ok(generated)
}

pub fn bind_attr(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Real expansion happens in `namespace`; keep the attributed item unchanged.
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
            tramps.push(generate_execute_tramp(&ns, &bind, method)?);
            method.attrs = strip_methoddb_attrs(&method.attrs);
        }
    }

    Ok(quote! {
        #impl_item
        #(#tramps)*
    })
}
