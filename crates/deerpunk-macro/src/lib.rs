//! Proc-macros for the Deerpunk host (`methoddb` API tramps).

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
