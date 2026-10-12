use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Expr, ItemFn, LitStr, Meta, Token,
};

pub struct ExportAttr {
    pub name: LitStr,
    pub scope_flags: u32,
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

impl Parse for ExportAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name: Option<LitStr> = None;
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
                Meta::NameValue(nv) if nv.path.is_ident("scope") => {
                    scope_flags = Some(parse_scope_flags(&nv.value)?);
                }
                _ => {}
            }
        }

        Ok(ExportAttr {
            name: name.ok_or_else(|| syn::Error::new(input.span(), "missing `name = \"v1:…\"`"))?,
            scope_flags: scope_flags.ok_or_else(|| {
                syn::Error::new(
                    input.span(),
                    "missing `scope = [\"server\", \"client\"]` (or a subset)",
                )
            })?,
        })
    }
}

pub fn export_attr(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let export: ExportAttr = syn::parse2(attr)?;
    let func: ItemFn = syn::parse2(item)?;
    let fn_name = &func.sig.ident;
    let api_name = export.name.value();
    let scope_flags = export.scope_flags;

    Ok(quote! {
        #func

        ::inventory::submit! {
            crate::engine_api::engine_api::EngineApiMethod {
                name: #api_name,
                callback: #fn_name as *const (),
                flags: #scope_flags,
            }
        }
    })
}
