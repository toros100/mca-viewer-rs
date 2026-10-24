mod derive_deserialize;
mod derive_serialize;
mod util;

pub(crate) use util::*;

#[proc_macro_derive(DeserializePayload, attributes(nbt))]
pub fn derive_deserialize_payload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive_deserialize::derive_deserialize_payload_inner(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[proc_macro_derive(SerializePayload, attributes(nbt))]
pub fn derive_serialize_payload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive_serialize::derive_serialize_payload_inner(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
