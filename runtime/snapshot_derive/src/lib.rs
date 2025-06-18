use proc_macro::TokenStream;
use snapshot_derive_impl::{
    derive_deserialize_impl, derive_serialize_impl, derive_serialize_mut_impl,
};

#[proc_macro_derive(Serialize)]
pub fn derive_serialize(input: TokenStream) -> TokenStream {
    derive_serialize_impl(input.into()).into()
}

#[proc_macro_derive(SerializeMut)]
pub fn derive_serialize_mut(input: TokenStream) -> TokenStream {
    derive_serialize_mut_impl(input.into()).into()
}

#[proc_macro_derive(Deserialize)]
pub fn derive_deserialize(input: TokenStream) -> TokenStream {
    derive_deserialize_impl(input.into()).into()
}
