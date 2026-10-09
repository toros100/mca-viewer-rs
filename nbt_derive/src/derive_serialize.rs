use crate::process_fields;
use quote::{quote, quote_spanned};
use syn::DeriveInput;
use syn::spanned::Spanned;

pub fn derive_serialize_payload_inner(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let fields = match &input.data {
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

    let fields = process_fields(fields)?;

    let mut field_names = Vec::<String>::new();
    let mut serialize_calls = Vec::<proc_macro2::TokenStream>::new();
    let mut impl_serialize_asserts = Vec::<proc_macro2::TokenStream>::new();
    let mut impl_optional_asserts = Vec::<proc_macro2::TokenStream>::new();

    let assert_impl_serialize_fn = quote! {
        fn _assert_impl_serialize<T: ::nbt::SerializePayload>(){}
    };

    let assert_impl_optional_fn = quote! {
        fn _assert_impl_optional<T: ::nbt::Optional>(){}
    };

    for f in fields.iter() {
        if f.optional {
            let ty = &f.field.ty;
            impl_optional_asserts
                .push(quote_spanned! {ty.span() => _assert_impl_optional::<#ty>();});
        }

        let field_ident = f.field.ident.clone().unwrap();

        let ty = &f.field.ty;
        impl_serialize_asserts.push(quote_spanned! { ty.span() =>
            _assert_impl_serialize::<#ty>();
        });

        let field_name = f.name_string.clone();

        let field_str_bytes =
            syn::LitByteStr::new(field_name.as_bytes(), proc_macro2::Span::call_site());

        let len = field_name.len() as u16;

        field_names.push(field_name);

        if f.optional {
            serialize_calls.push(quote! {
                if ::nbt::Optional::is_present(&self.#field_ident) {
                    buf.push(<#ty as ::nbt::SerializePayload>::TAG as u8);
                    buf.extend_from_slice(&(#len).to_be_bytes());
                    buf.extend_from_slice(#field_str_bytes);
                    self.#field_ident.serialize_payload(buf)?;
                }
            });
        } else {
            serialize_calls.push(quote! {
                buf.push(<#ty as ::nbt::SerializePayload>::TAG as u8);
                buf.extend_from_slice(&(#len).to_be_bytes());
                buf.extend_from_slice(#field_str_bytes);
                self.#field_ident.serialize_payload(buf)?;
            });
        }
    }

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {

        impl #impl_generics ::nbt::SerializePayload for #name #ty_generics #where_clause {

            const TAG: ::nbt::Tag = ::nbt::Tag::Compound;

            fn serialize_payload(&self, buf: &mut Vec<u8>) -> ::nbt::SerializationResult<()> {

                #assert_impl_serialize_fn
                #(#impl_serialize_asserts)*

                #assert_impl_optional_fn
                #(#impl_optional_asserts)*


                #(#serialize_calls)*

                buf.push(::nbt::Tag::End as u8);
                Ok(())
            }
        }

    })
}
