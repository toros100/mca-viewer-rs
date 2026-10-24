use crate::{
    LifetimeSubstitution, get_borrow_lifetime, insert_lifetime, parse_struct_attrs, process_fields,
};
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
    let attrs = parse_struct_attrs(&input.attrs)?;

    let exit_early = attrs.exit_early;

    let mut cases: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut post_check: Vec<proc_macro2::TokenStream> = vec![];

    for (i, f) in fields.iter().enumerate() {
        let field_ident = f.field.ident.clone().unwrap();

        let field_str = &f.name_string;

        let field_str_bytes =
            syn::LitByteStr::new(f.name_string.as_bytes(), proc_macro2::Span::call_site());
        let ty = &f.field.ty;

        if f.optional {
            post_check.push(quote! {
                if !found_fields[#i] {
                    ::nbt::Optional::handle_absent(&mut self.#field_ident);
                }
            })
        } else {
            post_check.push(quote! {
                if !found_fields[#i] {
                    return Err(::nbt::DeserializationError::FieldNotFound(#field_str));
                }
            })
        }

        cases.push(quote! {
            #field_str_bytes => {
                if found_fields[#i] {
                    return Err(::nbt::DeserializationError::FieldDuplicate(#field_str))
                };

                if <#ty as ::nbt::DeserializePayload>::TAG != t {
                    return Err(::nbt::DeserializationError::UnexpectedTag{
                        expected: <#ty as ::nbt::DeserializePayload>::TAG,
                        found: t,
                    })
                };

                off += ::nbt::DeserializePayload::deserialize_payload(&mut self.#field_ident, &data[off..])?;

                found_fields[#i] = true;
                if EXIT_EARLY {
                    remaining_fields -= 1;
                    if remaining_fields == 0 {
                        return Ok(off);
                    }
                }
            }
        });
    }

    cases.push(quote! {
        _ => {off += ::nbt::skip_payload(&data[off..], t)?;}
    });

    let num_fields = fields.len();

    let generics_cl_1 = input.generics.clone();
    let mut generics_cl_2 = input.generics.clone();
    let borrow_lt = syn::Lifetime::new("'__b", proc_macro2::Span::call_site());

    let (trait_lt, impl_generics, ty_generics, where_clause, _borrow_ty_generics) = {
        match get_borrow_lifetime(&input)? {
            Some(lp) => {
                let trait_lt = lp.lifetime.clone();

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

    Ok(quote! {
        impl #impl_generics ::nbt::DeserializePayload<#trait_lt> for #name #ty_generics #where_clause {
            const TAG: ::nbt::Tag = ::nbt::Tag::Compound;

            fn deserialize_payload(&mut self, data: &#trait_lt [u8]) -> ::nbt::DeserializationResult<usize> {

                #assert_impl_optional_fn
                #(#impl_optional_asserts)*

                const NUM_FIELDS : usize = #num_fields;
                const EXIT_EARLY : bool = #exit_early;
                const FIELD_OPTIONAL: [bool; NUM_FIELDS] = [#(#is_optional_bools),*];

                let data_len = data.len();
                let mut off = 0;
                let mut found_fields = [false; NUM_FIELDS];
                let mut remaining_fields = NUM_FIELDS;

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

                #(#post_check)*


                Ok(off)
            }
        }
    })
}
