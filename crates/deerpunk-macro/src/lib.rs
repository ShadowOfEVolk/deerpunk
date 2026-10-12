//! Proc-macros for the Deerpunk host (`methoddb` API tramps).

mod export;
mod methoddb;

use proc_macro::TokenStream;

/// Mark a method for FFI tramp generation (processed by [`namespace`] on the enclosing `impl`).
#[proc_macro_attribute]
pub fn bind(attr: TokenStream, item: TokenStream) -> TokenStream {
    methoddb::bind_attr(attr.into(), item.into()).into()
}

/// `#[methoddb::namespace(name = "v1:cmd", host_field = "cmd", scope = ["server", "client"])]`
///
/// Generates `extern "C"` tramps + `inventory::submit!` for methods marked with [`bind`].
#[proc_macro_attribute]
pub fn namespace(attr: TokenStream, item: TokenStream) -> TokenStream {
    methoddb::namespace_attr(attr.into(), item.into()).into()
}

/// Register an existing `extern "C"` tramp in MethodDB (`inventory::submit!`).
///
/// `#[methoddb::export(name = "v1:net:client_cmd", scope = ["server", "client"])]`
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    match export::export_attr(attr.into(), item.into()) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
