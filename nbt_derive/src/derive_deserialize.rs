use crate::{get_borrow_lifetime, parse_struct_attrs, process_fields};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{DeriveInput, parse_quote};

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

    // TODO: should ensure __C is a fresh identifier
    let ctx_ty = syn::Ident::new("__C", proc_macro2::Span::call_site());

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

                if <#ty as ::nbt::DeserializePayload::<#ctx_ty>>::TAG != t {
                    return Err(::nbt::DeserializationError::UnexpectedTag{
                        expected: <#ty as ::nbt::DeserializePayload::<#ctx_ty>>::TAG,
                        found: t,
                    })
                };

                off += ::nbt::DeserializePayload::<#ctx_ty>::deserialize_payload(&mut self.#field_ident, &data[off..], ctx)?;

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

    let mut g = input.generics.clone();

    let (trait_lt, impl_generics, ty_generics, where_clause) = {
        let trait_lt = if let Some(lt) = get_borrow_lifetime(&input)? {
            lt.lifetime.clone()
        } else {
            // TODO: should ensure '__d is a fresh lifetime
            let trait_lt = syn::Lifetime::new("'__d", proc_macro2::Span::call_site());
            g.params.push(parse_quote!(#trait_lt));
            trait_lt
        };

        g.params.push(parse_quote!(#ctx_ty));

        let wc = g.make_where_clause();
        for f in fields.iter() {
            let ty = &f.field.ty;
            wc.predicates
                .push(parse_quote!(#ty: ::nbt::DeserializePayload<#trait_lt, #ctx_ty>));
        }

        let (impl_generics, _, where_clause) = g.split_for_impl();
        let (_, ty_generics, _) = input.generics.split_for_impl();

        (trait_lt, impl_generics, ty_generics, where_clause)
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
        impl #impl_generics ::nbt::DeserializePayload<#trait_lt, #ctx_ty> for #name #ty_generics #where_clause {
            const TAG: ::nbt::Tag = ::nbt::Tag::Compound;

            fn deserialize_payload(&mut self, data: &#trait_lt [u8], ctx: &mut #ctx_ty) -> ::nbt::DeserializationResult<usize> {

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
