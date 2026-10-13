use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse::Parse, parse::ParseStream, punctuated::Punctuated, Ident, Token, Type,
};

struct Input {
    api: Ident,
    types: Punctuated<Type, Token![,]>,
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let api: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let types = Punctuated::parse_terminated(input)?;
        Ok(Input { api, types })
    }
}

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let Input { api, types } = syn::parse2(input)?;
    let mut regs = Vec::new();
    for ty in types {
        regs.push(quote! { #ty::register(#api) });
    }
    Ok(quote! {
        (#(#regs,)*)
    })
}
