//! `#[game_system]`: declare an ECS system next to its handler.
//!
//! The annotated function keeps its signature and body (valid Rust for rust-analyzer); the
//! macro appends the `extern "C"` FFI glue and an idempotent `pub fn register(api: &EngineApi)`
//! that registers the query components and the system itself.
//!
//! ```ignore
//! // SLICES mode: one `&[Component]` parameter per query term, in query order.
//! #[game_system(client, name = "client.chunk_draw", stage = "renderer.prepare", without(ChunkCulled))]
//! fn chunk_draw(
//!     api: &EngineApi,                 // optional special parameter
//!     chunks: &[ChunkRenderer],
//!     xforms: &[WorldTransformComponent],
//!     entities: &[EntityId],           // optional special parameter
//!     dt: f32,                         // optional special parameter
//! ) { ... }
//!
//! // ROWS mode: exactly one rows parameter instead of component slices.
//! #[game_system(server, name = "sim.gravity")]
//! fn gravity(dt: f32, rows: impl Iterator<Item = (EntityId, &Velocity, &WorldTransformComponent)>) { ... }
//!
//! #[game_system(server, name = "sim.gravity")]
//! fn gravity(dt: f32, rows: Rows<'_, (EntityId, &Velocity, &WorldTransformComponent)>) { ... }
//! ```
//!
//! The `Rows` alias is generated into the annotated function's module (one ROWS-mode system
//! per module). Its tuple argument is documentation for humans: the type system ignores it,
//! the macro derives the query and the glue from it.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::Parse, parse::ParseStream, punctuated::Punctuated, Expr, FnArg, GenericArgument, Lit,
    Meta, Pat, PathArguments, Token, Type, TypeParamBound,
};

/// Parsed `#[game_system(...)]` arguments.
struct SystemArgs {
    client: bool,
    name: String,
    stage: Option<String>,
    interval: Option<Expr>,
    without: Vec<Type>,
}

impl Parse for SystemArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut world: Option<bool> = None;
        let mut name: Option<String> = None;
        let mut stage: Option<String> = None;
        let mut interval: Option<Expr> = None;
        let mut without: Vec<Type> = Vec::new();
        for (idx, meta) in metas.into_iter().enumerate() {
            match meta {
                Meta::Path(p) if p.is_ident("client") || p.is_ident("server") => {
                    if idx != 0 {
                        return Err(syn::Error::new_spanned(
                            &p,
                            "client|server must be the first game_system argument",
                        ));
                    }
                    if world.replace(p.is_ident("client")).is_some() {
                        return Err(syn::Error::new_spanned(
                            &p,
                            "duplicate `client`/`server` in game_system",
                        ));
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    if name.is_some() {
                        return Err(syn::Error::new_spanned(
                            &nv,
                            "duplicate `name` in game_system",
                        ));
                    }
                    name = Some(string_value(&nv)?);
                }
                Meta::NameValue(nv) if nv.path.is_ident("stage") => {
                    if stage.is_some() {
                        return Err(syn::Error::new_spanned(
                            &nv,
                            "duplicate `stage` in game_system",
                        ));
                    }
                    stage = Some(string_value(&nv)?);
                }
                Meta::NameValue(nv) if nv.path.is_ident("interval") => {
                    if interval.is_some() {
                        return Err(syn::Error::new_spanned(
                            &nv,
                            "duplicate `interval` in game_system",
                        ));
                    }
                    interval = Some(nv.value);
                }
                Meta::List(list) if list.path.is_ident("without") => {
                    if !matches!(list.delimiter, syn::MacroDelimiter::Paren(_)) {
                        return Err(syn::Error::new_spanned(
                            &list,
                            "without(...) takes a comma-separated component list",
                        ));
                    }
                    let types =
                        list.parse_args_with(Punctuated::<Type, Token![,]>::parse_terminated)?;
                    without.extend(types);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown game_system attribute (client|server, name, stage, interval, without)",
                    ));
                }
            }
        }
        let client = world.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "game_system requires `client` or `server` as the first argument",
            )
        })?;
        let name = name
            .ok_or_else(|| syn::Error::new(input.span(), "game_system requires name = \"...\""))?;
        Ok(SystemArgs {
            client,
            name,
            stage,
            interval,
            without,
        })
    }
}

fn string_value(nv: &syn::MetaNameValue) -> syn::Result<String> {
    let Expr::Lit(expr_lit) = &nv.value else {
        return Err(syn::Error::new_spanned(
            &nv.value,
            "must be a string literal",
        ));
    };
    let Lit::Str(s) = &expr_lit.lit else {
        return Err(syn::Error::new_spanned(
            expr_lit,
            "must be a string literal",
        ));
    };
    Ok(s.value())
}

/// Final path segment of a plain type path, or `None` for any other type shape.
fn last_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) if p.qself.is_none() => Some(p.path.segments.last()?.ident.to_string()),
        _ => None,
    }
}

/// ECS name of a component type: the last path segment.
fn short_name(ty: &Type) -> syn::Result<String> {
    last_ident(ty).ok_or_else(|| {
        syn::Error::new_spanned(ty, "game_system component types must be type paths")
    })
}

/// How one annotated-function parameter maps onto the FFI callback.
enum Param {
    /// `api: &EngineApi` — resolved from the SDK's installed engine at call time.
    Api,
    /// `dt: f32` — forwarded from the FFI callback.
    Dt,
    /// `entities: &[EntityId]` — the entity id column parallel to the components.
    Entities,
    /// ROWS mode with the macro-provided `Rows<'_, (EntityId, &C1, …)>` type.
    RowsAlias { entity: Type, comps: Vec<Type> },
    /// ROWS mode with `impl Iterator<Item = (EntityId, &C1, …)>`.
    RowsIter { entity: Type, comps: Vec<Type> },
    /// SLICES mode: one `&[C]` column, in query order.
    Slice(Type),
}

struct Analysis {
    params: Vec<Param>,
    /// Query components in declaration order (the `fields[]` order, no `!excluded` terms).
    comps: Vec<Type>,
    /// ROWS mode: the tuple's first element (the `EntityId` type as written).
    entity: Option<Type>,
    uses_api: bool,
    uses_dt: bool,
    uses_entities: bool,
    rows_alias: bool,
}

fn analyze_params(sig: &syn::Signature) -> syn::Result<Analysis> {
    let mut params = Vec::new();
    let mut api = 0usize;
    let mut dt = 0usize;
    let mut entities = 0usize;
    let mut rows = 0usize;
    let mut slices = 0usize;
    for arg in &sig.inputs {
        let FnArg::Typed(pt) = arg else {
            return Err(syn::Error::new_spanned(
                arg,
                "game_system does not support self or variadic parameters",
            ));
        };
        let Pat::Ident(pat) = &*pt.pat else {
            return Err(syn::Error::new_spanned(
                &pt.pat,
                "game_system parameters must be plain identifiers",
            ));
        };
        if pat.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                &pt.pat,
                "game_system parameters must be plain identifiers",
            ));
        }
        let key = pat.ident.to_string();
        let key = key.trim_start_matches('_').to_string();
        let param = match key.as_str() {
            "api" => {
                expect_api_ref(&pt.ty)?;
                api += 1;
                Param::Api
            }
            "dt" => {
                if last_ident(&pt.ty).as_deref() != Some("f32") {
                    return Err(syn::Error::new_spanned(
                        &pt.ty,
                        "the `dt` parameter must be `f32`",
                    ));
                }
                dt += 1;
                Param::Dt
            }
            "entities" => {
                expect_entities_slice(&pt.ty)?;
                entities += 1;
                Param::Entities
            }
            _ => match classify_column(&pt.ty)? {
                Classified::RowsAlias { entity, comps } => {
                    rows += 1;
                    Param::RowsAlias { entity, comps }
                }
                Classified::RowsIter { entity, comps } => {
                    rows += 1;
                    Param::RowsIter { entity, comps }
                }
                Classified::Slice(ty) => {
                    slices += 1;
                    Param::Slice(ty)
                }
            },
        };
        params.push(param);
    }
    if api > 1 || dt > 1 || entities > 1 || rows > 1 {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            "game_system accepts at most one api, dt, entities and rows parameter",
        ));
    }
    if rows > 0 && slices > 0 {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            "game_system: do not mix a `rows: …` parameter with `&[Component]` parameters",
        ));
    }
    if rows == 0 && slices == 0 {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            "game_system needs one `rows: impl Iterator<Item = (EntityId, &C, …)>` / \
             `Rows<'_, (EntityId, &C, …)>` parameter or at least one `&[Component]` parameter",
        ));
    }

    let mut comps = Vec::new();
    let mut entity = None;
    let mut rows_alias = false;
    for param in &params {
        match param {
            Param::RowsAlias {
                entity: e,
                comps: c,
            } => {
                entity = Some(e.clone());
                comps = c.clone();
                rows_alias = true;
            }
            Param::RowsIter {
                entity: e,
                comps: c,
            } => {
                entity = Some(e.clone());
                comps = c.clone();
            }
            Param::Slice(ty) => comps.push(ty.clone()),
            _ => {}
        }
    }
    Ok(Analysis {
        params,
        comps,
        entity,
        uses_api: api == 1,
        uses_dt: dt == 1,
        uses_entities: entities == 1,
        rows_alias,
    })
}

fn expect_api_ref(ty: &Type) -> syn::Result<()> {
    let err = || syn::Error::new_spanned(ty, "the `api` parameter must be `&EngineApi`");
    let Type::Reference(r) = ty else {
        return Err(err());
    };
    if r.mutability.is_some() || last_ident(&r.elem).as_deref() != Some("EngineApi") {
        return Err(err());
    }
    Ok(())
}

fn expect_entities_slice(ty: &Type) -> syn::Result<()> {
    let err = || syn::Error::new_spanned(ty, "the `entities` parameter must be `&[EntityId]`");
    let Type::Reference(r) = ty else {
        return Err(err());
    };
    if r.mutability.is_some() {
        return Err(err());
    }
    let Type::Slice(s) = &*r.elem else {
        return Err(err());
    };
    if last_ident(&s.elem).as_deref() != Some("EntityId") {
        return Err(err());
    }
    Ok(())
}

enum Classified {
    RowsAlias { entity: Type, comps: Vec<Type> },
    RowsIter { entity: Type, comps: Vec<Type> },
    Slice(Type),
}

/// Distinguish modes by parameter type shape: `impl Iterator`/`Rows` vs `&[T]`.
fn classify_column(ty: &Type) -> syn::Result<Classified> {
    match ty {
        Type::ImplTrait(imp) => {
            for bound in &imp.bounds {
                let TypeParamBound::Trait(tr) = bound else {
                    continue;
                };
                let Some(seg) = tr.path.segments.last() else {
                    continue;
                };
                if seg.ident != "Iterator" {
                    continue;
                }
                let PathArguments::AngleBracketed(args) = &seg.arguments else {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "game_system: `impl Iterator` needs `Item = (EntityId, &C1, …)`",
                    ));
                };
                for arg in &args.args {
                    let GenericArgument::AssocType(assoc) = arg else {
                        continue;
                    };
                    if assoc.ident != "Item" {
                        continue;
                    }
                    let (entity, comps) = row_tuple(&assoc.ty)?;
                    return Ok(Classified::RowsIter { entity, comps });
                }
                return Err(syn::Error::new_spanned(
                    ty,
                    "game_system: `impl Iterator` needs `Item = (EntityId, &C1, …)`",
                ));
            }
            Err(syn::Error::new_spanned(
                ty,
                "game_system: impl-trait parameter must be `impl Iterator<Item = (EntityId, &C1, …)>`",
            ))
        }
        Type::Path(p) => match p.path.segments.last() {
            Some(seg) if p.qself.is_none() && seg.ident == "Rows" => {
                let PathArguments::AngleBracketed(args) = &seg.arguments else {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "game_system: `Rows` needs a tuple: `Rows<'_, (EntityId, &C1, …)>`",
                    ));
                };
                for arg in &args.args {
                    let GenericArgument::Type(item) = arg else {
                        continue;
                    };
                    let (entity, comps) = row_tuple(item)?;
                    return Ok(Classified::RowsAlias { entity, comps });
                }
                Err(syn::Error::new_spanned(
                    ty,
                    "game_system: `Rows` needs a tuple: `Rows<'_, (EntityId, &C1, …)>`",
                ))
            }
            _ => Err(syn::Error::new_spanned(ty, unsupported_params())),
        },
        Type::Reference(r) => {
            if r.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    ty,
                    "game_system component columns are read-only: use `&[Component]`",
                ));
            }
            let Type::Slice(s) = &*r.elem else {
                return Err(syn::Error::new_spanned(ty, unsupported_params()));
            };
            Ok(Classified::Slice((*s.elem).clone()))
        }
        _ => Err(syn::Error::new_spanned(ty, unsupported_params())),
    }
}

fn unsupported_params() -> &'static str {
    "game_system parameters are `&[Component]` slices, `rows: impl Iterator<Item = (EntityId, \
     &C, …)>` / `Rows<'_, (EntityId, &C, …)>`, or api/dt/entities"
}

/// Validate a `(EntityId, &C1, &C2, …)` rows item tuple; returns (entity, components).
fn row_tuple(ty: &Type) -> syn::Result<(Type, Vec<Type>)> {
    let Type::Tuple(tuple) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "game_system rows item must be a tuple `(EntityId, &C1, …)` (trailing comma for one component)",
        ));
    };
    let mut elems = tuple.elems.iter();
    let Some(entity) = elems.next() else {
        return Err(syn::Error::new_spanned(
            ty,
            "game_system rows item must be a tuple `(EntityId, &C1, …)`",
        ));
    };
    if last_ident(entity).as_deref() != Some("EntityId") {
        return Err(syn::Error::new_spanned(
            entity,
            "game_system rows item must start with `EntityId` by value",
        ));
    }
    let mut comps = Vec::new();
    for elem in elems {
        let Type::Reference(r) = elem else {
            return Err(syn::Error::new_spanned(
                elem,
                "game_system rows item components must be `&Component` references",
            ));
        };
        if r.mutability.is_some() {
            return Err(syn::Error::new_spanned(
                elem,
                "game_system component columns are read-only: use `&Component`",
            ));
        }
        comps.push((*r.elem).clone());
    }
    if comps.is_empty() {
        return Err(syn::Error::new_spanned(
            ty,
            "game_system rows item needs at least one `&Component`",
        ));
    }
    Ok((entity.clone(), comps))
}

/// Zip/map iterator over `(EntityId, &C1, …)` from the `__entities`/`__cN` glue locals.
/// No allocation: slices zipped lazily.
fn rows_iter_expr(comps: &[Type]) -> TokenStream {
    let mut pat = quote! { __e };
    for i in 0..comps.len() {
        let v = format_ident!("__x{}", i);
        pat = quote! { (#pat, #v) };
    }
    let xs: Vec<_> = (0..comps.len())
        .map(|i| format_ident!("__x{}", i))
        .collect();
    let cols: Vec<_> = (0..comps.len())
        .map(|i| format_ident!("__c{}", i))
        .collect();
    let mut iter = quote! { __entities.iter().copied() };
    for c in &cols {
        iter = quote! { #iter.zip(#c) };
    }
    quote! { #iter.map(|#pat| (__e, #( #xs, )*)) }
}

/// Iterator struct + `Rows` alias for the `Rows<'_, (EntityId, &C, …)>` parameter form.
///
/// The struct carries the annotated tuple as `PhantomData` (type aliases reject unused
/// parameters, so `Rows<'a, T>` must pass `T` through), while the blanket `Iterator` impl
/// yields the concrete tuple with one unified lifetime. The elided lifetimes inside the
/// user's tuple are independent function-level parameters, so the tuple stays documentation:
/// the macro derives query and glue from it, the type system iterates the unified item.
fn rows_type_items(rows_ident: &syn::Ident, entity: &Type, comps: &[Type]) -> TokenStream {
    let tuple_param = format_ident!("__T");
    let struct_fields = comps.iter().enumerate().map(|(i, ty)| {
        let f = format_ident!("__c{}", i);
        quote! { #f: ::std::slice::Iter<'a, #ty>, }
    });
    let col_params = comps.iter().enumerate().map(|(i, ty)| {
        let f = format_ident!("__c{}", i);
        quote! { #f: &'a [#ty], }
    });
    let col_inits = comps.iter().enumerate().map(|(i, _)| {
        let f = format_ident!("__c{}", i);
        quote! { #f: #f.iter(), }
    });
    let col_fields: Vec<_> = (0..comps.len())
        .map(|i| format_ident!("__c{}", i))
        .collect();
    quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #rows_ident<'a, #tuple_param> {
            __e: ::std::iter::Copied<::std::slice::Iter<'a, #entity>>,
            #(#struct_fields)*
            __pd: ::std::marker::PhantomData<#tuple_param>,
        }

        impl<'a, #tuple_param> #rows_ident<'a, #tuple_param> {
            fn new(__e: &'a [#entity], #(#col_params)*) -> Self {
                Self {
                    __e: __e.iter().copied(),
                    #(#col_inits)*
                    __pd: ::std::marker::PhantomData,
                }
            }
        }

        impl<'a, #tuple_param> ::std::iter::Iterator for #rows_ident<'a, #tuple_param> {
            type Item = (#entity, #( &'a #comps, )*);

            fn next(&mut self) -> ::std::option::Option<Self::Item> {
                ::std::option::Option::Some((
                    self.__e.next()?,
                    #( self.#col_fields.next()?, )*
                ))
            }

            fn size_hint(&self) -> (usize, ::std::option::Option<usize>) {
                let __n = self.__e.len()#( .min(self.#col_fields.len()) )*;
                (__n, ::std::option::Option::Some(__n))
            }
        }

        #[doc(hidden)]
        pub type Rows<'a, #tuple_param> = #rows_ident<'a, #tuple_param>;
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args: SystemArgs = syn::parse2(attr)?;
    let input: syn::ItemFn = syn::parse2(item)?;

    let mut bad_generics = None;
    for param in &input.sig.generics.params {
        if !matches!(param, syn::GenericParam::Lifetime(_)) {
            bad_generics = Some(param);
        }
    }
    if let Some(param) = bad_generics {
        return Err(syn::Error::new_spanned(
            param,
            "game_system does not support type or const generic parameters",
        ));
    }
    if let Some(asyn) = &input.sig.asyncness {
        return Err(syn::Error::new_spanned(
            asyn,
            "game_system does not support async functions",
        ));
    }
    if input.sig.ident == "register" {
        return Err(syn::Error::new_spanned(
            &input.sig.ident,
            "game_system generates `pub fn register`; rename this function",
        ));
    }

    let analysis = analyze_params(&input.sig)?;
    let fn_ident = &input.sig.ident;
    let glue_ident = format_ident!("{}_glue", fn_ident);
    let name_str = &args.name;
    let world = if args.client {
        quote! { api.client_world }
    } else {
        quote! { api.world }
    };

    // Component registrations cover the query terms, `without(...)` included (the engine keys
    // components by name, so repeated registration is harmless and keeps the `let _ =`
    // pattern of the handwritten systems).
    let mut reg_calls = Vec::new();
    let mut query_terms = Vec::new();
    for ty in &analysis.comps {
        let comp_name = short_name(ty)?;
        query_terms.push(comp_name.clone());
        reg_calls.push(quote! {
            let _ = #world.register_component(
                #comp_name,
                ::std::mem::size_of::<#ty>(),
                ::std::mem::align_of::<#ty>(),
            );
        });
    }
    for ty in &args.without {
        let comp_name = short_name(ty)?;
        query_terms.push(format!("!{comp_name}"));
        reg_calls.push(quote! {
            let _ = #world.register_component(
                #comp_name,
                ::std::mem::size_of::<#ty>(),
                ::std::mem::align_of::<#ty>(),
            );
        });
    }
    let query = query_terms.join(", ");

    let interval = match &args.interval {
        Some(expr) => quote! { #expr },
        None => quote! { 0.0 },
    };
    let stage_check = match &args.stage {
        Some(stage) => quote! {
            if #world.system_set_stage(id, #stage) != 0 {
                log::error!("[game_system] {}: system_set_stage failed id={id}", #name_str);
            }
        },
        None => quote! {},
    };

    // FFI glue: safe column slices (same checks as the handwritten systems), then a call with
    // the user's parameters in declaration order. No allocation: slices and zip/map iterators.
    let n = analysis.comps.len();
    let dt_param: TokenStream = if analysis.uses_dt {
        quote! { dt }
    } else {
        quote! { _dt }
    };
    let mut col_decls = Vec::new();
    for (i, ty) in analysis.comps.iter().enumerate() {
        let col = format_ident!("__c{}", i);
        col_decls.push(quote! {
            if __fields[#i].is_null() || __sizes[#i] != ::std::mem::size_of::<#ty>() {
                return;
            }
            let #col = unsafe {
                ::std::slice::from_raw_parts(__fields[#i] as *const #ty, __count)
            };
        });
    }
    let entities_decl = if analysis.entity.is_some() || analysis.uses_entities {
        quote! {
            let __entities = unsafe { ::std::slice::from_raw_parts(entities, __count) };
        }
    } else {
        quote! {}
    };
    let (rows_decl, rows_types) = match (&analysis.entity, analysis.rows_alias) {
        (Some(entity), true) => {
            let rows_ident = format_ident!("__game_system_rows_{}", fn_ident);
            let cols = (0..n).map(|i| format_ident!("__c{}", i));
            (
                quote! {
                    let __rows = #rows_ident::new(__entities, #( #cols, )*);
                },
                rows_type_items(&rows_ident, entity, &analysis.comps),
            )
        }
        (Some(_), false) => {
            let expr = rows_iter_expr(&analysis.comps);
            (
                quote! {
                    let __rows = #expr;
                },
                quote! {},
            )
        }
        (None, _) => (quote! {}, quote! {}),
    };
    let api_decl = if analysis.uses_api {
        quote! {
            let Some(api) = deerpunk_engine_sdk::try_engine() else {
                return;
            };
        }
    } else {
        quote! {}
    };
    let mut call_args = Vec::new();
    let mut slice_idx = 0usize;
    for param in &analysis.params {
        match param {
            Param::Api => call_args.push(quote! { api }),
            Param::Dt => call_args.push(quote! { dt }),
            Param::Entities => call_args.push(quote! { __entities }),
            Param::RowsAlias { .. } | Param::RowsIter { .. } => call_args.push(quote! { __rows }),
            Param::Slice(_) => {
                let col = format_ident!("__c{}", slice_idx);
                slice_idx += 1;
                call_args.push(quote! { #col });
            }
        }
    }
    // `quote!` renders `usize` with a suffix; the FFI guard compares against `u32`.
    let n_lit = proc_macro2::Literal::u32_unsuffixed(n as u32);
    let glue = quote! {
        extern "C" fn #glue_ident(
            entities: *const u64,
            count: u32,
            fields: *const *const ::std::ffi::c_void,
            field_count: u32,
            sizes: *const usize,
            #dt_param: f32,
            _user: *mut ::std::ffi::c_void,
        ) {
            if entities.is_null() || count == 0 || fields.is_null() || sizes.is_null()
                || field_count < #n_lit
            {
                return;
            }
            let __count = count as usize;
            let __fields = unsafe { ::std::slice::from_raw_parts(fields, field_count as usize) };
            let __sizes = unsafe { ::std::slice::from_raw_parts(sizes, field_count as usize) };
            #(#col_decls)*
            #entities_decl
            #rows_decl
            #api_decl
            #fn_ident(#(#call_args,)*);
        }
    };

    let register = quote! {
        pub fn register(api: &deerpunk_engine_sdk::EngineApi) {
            static ONCE: ::std::sync::Once = ::std::sync::Once::new();
            #(#reg_calls)*
            ONCE.call_once(|| {
                let id = #world.system_register(
                    #name_str,
                    #query,
                    #glue_ident,
                    ::std::ptr::null_mut(),
                    #interval,
                );
                #stage_check
            });
        }
    };

    Ok(quote! {
        #input
        #rows_types
        #glue
        #register
    })
}
