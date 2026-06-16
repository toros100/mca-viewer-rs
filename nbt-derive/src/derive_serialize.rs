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
    let mut impl_asserts = Vec::<proc_macro2::TokenStream>::new();

    let impl_assert_fn = quote! {
        fn _assert_impl<T: SerializePayload>(){}
    };

    for f in fields.iter() {
        let field_ident = f.field.ident.clone().unwrap();

        let ty = &f.field.ty;
        impl_asserts.push(quote_spanned! { ty.span() =>
            _assert_impl::<#ty>();
        });

        let field_name = f.name_string.clone();

        let field_str_bytes =
            syn::LitByteStr::new(field_name.as_bytes(), proc_macro2::Span::call_site());

        let len = field_name.len() as u16;

        field_names.push(field_name);

        serialize_calls.push(quote! {
            buf.push(<#ty as SerializePayload>::TAG as u8);
            buf.extend_from_slice(&(#len).to_be_bytes());
            buf.extend_from_slice(#field_str_bytes);
            self.#field_ident.serialize_into(buf)?;
        });
    }

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {

        impl #impl_generics SerializePayload for #name #ty_generics #where_clause {

            fn serialize_into(&self, buf: &mut Vec<u8>) -> std::result::Result<(), ::nbt::SerializationError> {

                #impl_assert_fn;
                #(#impl_asserts)*

                #(#serialize_calls)*

                buf.push(::nbt::Tag::End as u8);
                Ok(())
            }
        }

    })
}
