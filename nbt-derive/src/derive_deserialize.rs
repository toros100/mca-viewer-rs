use crate::{LifetimeSubstitution, get_borrow_lifetime, insert_lifetime, process_fields};
use quote::{ToTokens, quote, quote_spanned};
use syn::DeriveInput;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;

pub fn derive_deserialize_payload_inner(
    input: DeriveInput,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let fs = match &input.data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new(
                    input.ident.span(),
                    "only named fields supported",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "only structs supported",
            ));
        }
    };

    let fields = process_fields(fs)?;

    let mut cases: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut field_names: Vec<String> = Vec::new();

    for (i, f) in fields.iter().enumerate() {
        let field_ident = f.field.ident.clone().unwrap();

        let field_str = &f.name_string;

        field_names.push(field_str.clone());

        let field_str_bytes =
            syn::LitByteStr::new(f.name_string.as_bytes(), proc_macro2::Span::call_site());
        let ty = &f.field.ty;

        cases.push(quote! {
            #field_str_bytes => {
                if found_fields[#i] {
                    return Err(::nbt::DeserializationError::FieldDuplicate(#field_str))
                };

                if <#ty as ::nbt::DeserializePayload>::TAG != t {
                    return Err(::nbt::DeserializationError::UnexpectedTag(t as u8))
                };

                off += self.#field_ident.deserialize_payload(&data[off..])?;
                found_fields[#i] = true;
            }
        });
    }

    cases.push(quote! {
        _ => {off += ::nbt::skip_payload(&data[off..], t)?;}
    });

    let num_fields = fields.len();

    // input example:
    // struct Data<'a, T> { ... }
    //
    // 'a is assumed to be the lifetime to borrow from the bytes with
    // currently, the implementing type must have 0 or 1 lifetime param
    //
    // unsafe impl<'__d, T> DeserializePayload<'__d> Data<'__d, T> {
    //               ^                                      ^
    //        fresh lt param          fresh lt param substituted for borrow lt
    //
    //      type Borrow<'__b> = Data<'__b, T>;
    //                    ^            ^
    //        other fresh lt param     analogous substitution
    //      ...
    //      fn deserialize_payload(&mut self, &'__d [u8]) ...
    // }
    //
    //
    // input type without lt param:
    //
    // struct Data2<T> { ... }
    //
    // unsafe impl<'__d, T> DeserializePayload<'__d> Data2<T> {
    //               ^
    //        fresh lt param
    //
    //      type Borrow<'__b> = Data2<T>;
    //                    ^           ^
    //        other fresh lt param    does not occur in rhs
    //      ...
    //      fn deserialize_payload(&mut self, &'__d [u8]) ...
    // }
    //

    // let '__d and '__b be fresh lifetimes
    // if the input does not borrow:
    // 1. insert '__d into impl generics
    // 2. cool?
    //
    // if the input does borrow with 'a:
    // 1. replace 'a with '__d for the impl + ty + where. '__d should already be in the impl
    //      generics now.
    // 2. replace 'a with '__b for the borrow ty generics

    let generics_cl_1 = input.generics.clone();
    let mut generics_cl_2 = input.generics.clone();
    // let trait_lt = syn::Lifetime::new("'__d", proc_macro2::Span::call_site());
    let borrow_lt = syn::Lifetime::new("'__b", proc_macro2::Span::call_site());

    let (trait_lt, impl_generics, ty_generics, where_clause, _borrow_ty_generics) = {
        match get_borrow_lifetime(&input)? {
            Some(lp) => {
                let trait_lt = lp.lifetime.clone();
                // let mut subst_trait_lt =
                //     LifetimeSubstitution::new(lp.lifetime.clone(), trait_lt.clone());
                // subst_trait_lt.visit_generics_mut(&mut generics_cl_1);

                let mut subst_borrow_lt =
                    LifetimeSubstitution::new(lp.lifetime.clone(), borrow_lt.clone());
                subst_borrow_lt.visit_generics_mut(&mut generics_cl_2);

                let (impl_generics, ty_generics, where_clause) = generics_cl_1.split_for_impl();

                let (_, borrow_ty_generics, _) = generics_cl_2.split_for_impl();

                (
                    trait_lt,
                    impl_generics.into_token_stream(),
                    ty_generics,
                    where_clause,
                    borrow_ty_generics,
                )
            }
            None => {
                let trait_lt = syn::Lifetime::new("'__d", proc_macro2::Span::call_site());
                let (impl_generics, ty_generics, where_clause) = generics_cl_1.split_for_impl();

                let borrow_ty_generics = ty_generics.clone();

                (
                    trait_lt.clone(),
                    insert_lifetime(impl_generics, trait_lt),
                    ty_generics,
                    where_clause,
                    borrow_ty_generics,
                )
            }
        }
    };

    // let borrow_lt_param = get_borrow_lifetime(&input)?;
    // let borrow_ty_generics = {
    //     let g = match borrow_lt_param {
    //         Some(l) => {
    //             let mut subst = LifetimeSubstitution::new(l.lifetime.clone(), borrow_lt.clone());
    //             let mut generics = input.generics.clone();
    //             subst.visit_generics_mut(&mut generics);
    //             generics
    //         }
    //         None => input.generics.clone(),
    //     };
    //
    //     let (_, ty_generics_subst, _) = g.split_for_impl();
    //     ty_generics_subst.into_token_stream()
    // };

    // let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let assert_impl_optional_fn = quote! {fn _assert_impl_optional<T: ::nbt::Optional>(){}};
    let mut impl_optional_asserts = Vec::<proc_macro2::TokenStream>::new();
    let mut is_optional_bools = Vec::<proc_macro2::TokenStream>::new();

    for f in fields.iter() {
        let opt = &f.optional;
        if *opt {
            let ty = &f.field.ty;
            impl_optional_asserts
                .push(quote_spanned! {ty.span() => _assert_impl_optional::<#ty>();});
        }

        is_optional_bools.push(quote! {#opt});
    }

    let _sanitize_calls = fields.iter().map(|f| {
        let field_ident = f.field.ident.clone().unwrap();
        quote! {
            self.#field_ident.sanitize();
        }
    });

    // type Borrow<#borrow_lt> = #name #borrow_ty_generics;
    Ok(quote! {
        impl #impl_generics ::nbt::DeserializePayload<#trait_lt> for #name #ty_generics #where_clause {
            const TAG: ::nbt::Tag = ::nbt::Tag::Compound;

            fn deserialize_payload(&mut self, data: &#trait_lt [u8]) -> ::nbt::DeserializationResult<usize> {

                #assert_impl_optional_fn
                #(#impl_optional_asserts)*

                static FIELD_NAMES: &[&str] = &[#(#field_names),*];
                const NUM_FIELDS : usize = #num_fields;
                let field_optional : [bool; NUM_FIELDS] = [#(#is_optional_bools),*];
                let data_len = data.len();
                let mut off = 0;
                let mut found_fields = [false; NUM_FIELDS];

                loop {
                    if off >= data_len {
                        return Err(::nbt::DeserializationError::EOF);
                    };

                    let t = ::nbt::Tag::try_from(data[off])?;
                    off += 1;

                    if t == ::nbt::Tag::End {
                        break;
                    }

                    let (field_name, k) = ::nbt::deserialize_nbt_str_bytes(&data[off..])?;
                    off += k;

                    match field_name {
                        #(#cases)*
                    }
                }

                for (i, v) in found_fields.iter().enumerate() {
                    if !v && !field_optional[i] {
                        return Err(::nbt::DeserializationError::FieldNotFound(FIELD_NAMES[i]));
                    }
                }


                Ok(off)
            }
        }
    })
}
