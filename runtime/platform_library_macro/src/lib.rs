#[allow(unused_extern_crates)]
extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, parse_macro_input};

/// `platform` is a marker attribute for FFI codegen.
#[proc_macro_attribute]
pub fn platform(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    quote! {
        #[allow(clippy::needless_pass_by_value)]
        #item
    }
    .into()
}
