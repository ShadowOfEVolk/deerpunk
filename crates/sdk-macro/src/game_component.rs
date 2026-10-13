use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::Parse, parse::ParseStream, punctuated::Punctuated, Data, DeriveInput, Expr, Lit, Meta,
    Token, Type,
};

struct Attrs {
    client: bool,
    both: bool,
    name: Option<String>,
    prefab: bool,
    inspect: bool,
    schema: Option<Expr>,
    layout: Option<Type>,
}

impl Parse for Attrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut client = false;
        let mut both = false;
        let mut name = None;
        let mut prefab = false;
        let mut inspect = false;
        let mut schema = None;
        let mut layout = None;
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            match meta {
                Meta::Path(p) if p.is_ident("client") => client = true,
                Meta::Path(p) if p.is_ident("both") => both = true,
                Meta::Path(p) if p.is_ident("prefab") => prefab = true,
                Meta::Path(p) if p.is_ident("inspect") => inspect = true,
                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    let Expr::Lit(expr_lit) = nv.value else {
                        return Err(syn::Error::new_spanned(
                            nv.value,
                            "name must be a string literal",
                        ));
                    };
                    let Lit::Str(s) = expr_lit.lit else {
                        return Err(syn::Error::new_spanned(
                            expr_lit,
                            "name must be a string literal",
                        ));
                    };
                    name = Some(s.value());
                }
                Meta::NameValue(nv) if nv.path.is_ident("schema") => schema = Some(nv.value),
                Meta::NameValue(nv) if nv.path.is_ident("layout") => {
                    let mut tokens = TokenStream::new();
                    nv.value.to_tokens(&mut tokens);
                    layout = Some(syn::parse2(tokens)?);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown game_component attribute (client, both, prefab, inspect, name, schema, layout)",
                    ));
                }
            }
        }
        if client && both {
            return Err(syn::Error::new(
                input.span(),
                "use either `client` or `both`, not both",
            ));
        }
        Ok(Attrs {
            client,
            both,
            name,
            prefab,
            inspect,
            schema,
            layout,
        })
    }
}

fn inventory_submit(ident: &syn::Ident, name: &str) -> TokenStream {
    quote! {
        ::inventory::submit! {
            deerpunk_engine_sdk::GameComponentRegistrar {
                name: #name,
                register: #ident::register,
            }
        }
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs: Attrs = syn::parse2(attr)?;
    let input: DeriveInput = syn::parse2(item)?;
    let ident = &input.ident;
    let name = attrs.name.unwrap_or_else(|| ident.to_string());
    let inv = inventory_submit(ident, &name);

    let both = attrs.both;
    let client = attrs.client;
    let prefab = attrs.prefab;
    let inspect = attrs.inspect;

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "game_component does not support generic structs",
        ));
    }

    if !matches!(input.data, Data::Struct(_)) {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "game_component requires a struct",
        ));
    }

    let layout = attrs
        .layout
        .map(|t| quote! { #t })
        .unwrap_or_else(|| quote! { Self });

    let size_align = quote! {
        let __size = ::core::mem::size_of::<#layout>();
        let __align = ::core::mem::align_of::<#layout>();
    };

    let register_simple = quote! {
        #size_align
        let __name = Self::COMPONENT_NAME;
        if #both {
            let _ = api.client_world.register_component(__name, __size, __align);
            api.world.register_component(__name, __size, __align).unwrap_or(0)
        } else if #client {
            api.client_world.register_component(__name, __size, __align).unwrap_or(0)
        } else {
            api.world.register_component(__name, __size, __align).unwrap_or(0)
        }
    };

    let world_api = if both || !client {
        quote! { api.world }
    } else {
        quote! { api.client_world }
    };

    let json_trait = quote! { deerpunk_engine_sdk::InspectDump };

    if !prefab && !inspect {
        return Ok(quote! {
            #input

            impl #ident {
                pub const COMPONENT_NAME: &'static str = #name;

                pub fn register(api: &deerpunk_engine_sdk::EngineApi) -> deerpunk_engine_sdk::ComponentId {
                    #register_simple
                }
            }

            #inv
        });
    }

    let apply_fn = format_ident!("__apply_{}", ident.to_string().to_lowercase());
    let dump_fn = format_ident!("__inspect_dump_{}", ident.to_string().to_lowercase());

    let apply_fn_impl = if prefab {
        quote! {
            extern "C" fn #apply_fn(
                user: *mut ::std::ffi::c_void,
                entity: u64,
                component_id: u64,
                cbor_ptr: *const u8,
                cbor_len: usize,
            ) -> i32 {
                if user.is_null() {
                    return -1;
                }
                let api = unsafe { &*(user as *const deerpunk_engine_sdk::EngineApi) };
                let value = if cbor_ptr.is_null() || cbor_len == 0 {
                    ::serde_json::Value::Object(::serde_json::Map::new())
                } else {
                    let bytes = unsafe { ::std::slice::from_raw_parts(cbor_ptr, cbor_len) };
                    match deerpunk_engine_sdk::prefab::value_from_cbor(bytes) {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("[prefab] {} CBOR: {e}", #name);
                            return -1;
                        }
                    }
                };
                let component =
                    <Self as deerpunk_engine_sdk::PrefabFromValue>::from_prefab_value(&value);
                #world_api.set(entity, component_id, component);
                0
            }
        }
    } else {
        quote! {}
    };

    let dump_fn_impl = if inspect {
        quote! {
            extern "C" fn #dump_fn(
                user: *mut ::std::ffi::c_void,
                entity: u64,
                component_id: u64,
                out_ptr: *mut u8,
                out_cap: usize,
            ) -> i32 {
                if user.is_null() || out_ptr.is_null() || out_cap == 0 {
                    return -1;
                }
                let api = unsafe { &*(user as *const deerpunk_engine_sdk::EngineApi) };
                let component = match #world_api.get::<Self>(entity, component_id) {
                    Some(v) => v,
                    None => return -1,
                };
                let json = <Self as #json_trait>::to_inspect_json(&component);
                let cbor = match deerpunk_engine_sdk::prefab::value_to_cbor(&json) {
                    Ok(b) => b,
                    Err(_) => return -1,
                };
                if cbor.len() > out_cap {
                    return -1;
                }
                unsafe {
                    ::std::ptr::copy_nonoverlapping(cbor.as_ptr(), out_ptr, cbor.len());
                }
                cbor.len() as i32
            }
        }
    } else {
        quote! {}
    };

    let schema_static = if inspect {
        if let Some(schema_expr) = &attrs.schema {
            let cbor_init = if let Expr::Lit(expr_lit) = schema_expr {
                if let Lit::Str(s) = &expr_lit.lit {
                    let raw = s.value();
                    quote! {
                        static CBOR: ::std::sync::LazyLock<Vec<u8>> = ::std::sync::LazyLock::new(|| {
                            let value: ::serde_json::Value =
                                ::serde_json::from_str(#raw).expect("game_component schema JSON");
                            deerpunk_engine_sdk::prefab::value_to_cbor(&value)
                                .expect("game_component schema CBOR")
                        });
                    }
                } else {
                    quote! {
                        static CBOR: ::std::sync::LazyLock<Vec<u8>> = ::std::sync::LazyLock::new(|| {
                            deerpunk_engine_sdk::prefab::value_to_cbor(&(#schema_expr))
                                .expect("game_component schema CBOR")
                        });
                    }
                }
            } else {
                quote! {
                    static CBOR: ::std::sync::LazyLock<Vec<u8>> = ::std::sync::LazyLock::new(|| {
                        deerpunk_engine_sdk::prefab::value_to_cbor(&(#schema_expr))
                            .expect("game_component schema CBOR")
                    });
                }
            };
            quote! {
                fn __inspect_schema_cbor() -> &'static [u8] {
                    #cbor_init
                    CBOR.as_slice()
                }
            }
        } else {
            quote! {
                fn __inspect_schema_cbor() -> &'static [u8] {
                    static CBOR: ::std::sync::LazyLock<Vec<u8>> = ::std::sync::LazyLock::new(|| {
                        deerpunk_engine_sdk::inspect::schema_cbor::<#ident>()
                    });
                    CBOR.as_slice()
                }
            }
        }
    } else {
        quote! {
            fn __inspect_schema_cbor() -> &'static [u8] {
                &[]
            }
        }
    };

    let apply_fn_ptr = if prefab {
        quote! { Self::#apply_fn }
    } else {
        quote! { deerpunk_engine_sdk::no_prefab_apply() }
    };

    let dump_fn_ptr = if inspect {
        quote! { Self::#dump_fn }
    } else {
        quote! { deerpunk_engine_sdk::no_inspect_dump() }
    };

    let register_full = quote! {
        #size_align
        let schema = Self::__inspect_schema_cbor();
        let user = (api as *const deerpunk_engine_sdk::EngineApi) as *mut ::std::ffi::c_void;
        let desc = deerpunk_engine_sdk::ComponentRegisterDesc {
            name_ptr: Self::COMPONENT_NAME.as_ptr(),
            name_len: Self::COMPONENT_NAME.len(),
            size: __size,
            align: __align,
            user_data: user,
            apply_fn: #apply_fn_ptr,
            inspect_schema_ptr: schema.as_ptr(),
            inspect_schema_len: schema.len(),
            inspect_dump_user: user,
            inspect_dump_fn: #dump_fn_ptr,
        };
        if #both {
            let _ = api.client_world.component_register(&desc);
            api.world.component_register(&desc).unwrap_or(0)
        } else if #client {
            api.client_world.component_register(&desc).unwrap_or(0)
        } else {
            api.world.component_register(&desc).unwrap_or(0)
        }
    };

    Ok(quote! {
        #input

        impl #ident {
            pub const COMPONENT_NAME: &'static str = #name;

            #apply_fn_impl
            #dump_fn_impl
            #schema_static

            pub fn register(api: &deerpunk_engine_sdk::EngineApi) -> deerpunk_engine_sdk::ComponentId {
                #register_full
            }
        }

        #inv
    })
}
