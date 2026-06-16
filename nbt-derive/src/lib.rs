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

// #[proc_macro_derive(DeserializeReuse, attributes(nbt))]
// pub fn derive_deserialize_reuse(input: TokenStream) -> TokenStream {
//     let input = parse_macro_input!(input as DeriveInput);
//     derive_deserialize_reuse_inner(input)
//         .unwrap_or_else(|e| e.to_compile_error())
//         .into()
// }
//
// fn derive_deserialize_reuse_inner(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
//     let name = &input.ident;
//     let borrow_lt = syn::Lifetime::new("'__b", proc_macro2::Span::call_site());
//
//     let generics_cl_1 = input.generics.clone();
//     let (impl_generics, ty_generics, where_clause) = generics_cl_1.split_for_impl();
//
//     let mut generics_cl_2 = input.generics.clone();
//     let borrow_ty_generics = match get_borrow_lifetime(&input)? {
//         Some(lp) => {
//             let mut subst_borrow_lt =
//                 LifetimeSubstitution::new(lp.lifetime.clone(), borrow_lt.clone());
//             subst_borrow_lt.visit_generics_mut(&mut generics_cl_2);
//
//             let (_, borrow_ty_generics, _) = generics_cl_2.split_for_impl();
//             Some(borrow_ty_generics)
//         }
//         None => None,
//     };
//
//     let borrow_ty_generics = borrow_ty_generics.unwrap_or(ty_generics.clone());
//
//     Ok(quote! {
//         unsafe impl #impl_generics ::nbt::DeserializeReuse for #name #ty_generics #where_clause {
//             type Borrow<#borrow_lt> = #name #borrow_ty_generics;
//
//         }
//     })
// }
