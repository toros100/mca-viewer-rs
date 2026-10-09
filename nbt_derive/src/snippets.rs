// random snippets of code that i no longer need but want to keep around for reference


#[proc_macro_attribute]
pub fn tagged(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_cl = input.clone();
    let derive_input = parse_macro_input!(input_cl as DeriveInput);
    tagged_inner(args, input, derive_input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn to_tag(args: TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    const ERR_MSG: &str = "Expected ::nbt::Tag variant identifier (excluding End)";
    match args.into_iter().next() {
        Some(proc_macro::TokenTree::Ident(ident)) => {
            let s = ident.to_string();
            match s.as_str() {
                // kinda hacky
                "Byte" | "Short" | "Int" | "Float" | "Double" | "String" | "List" | "ByteArray"
                | "Compound" | "IntArray" | "LongArray" => {
                    // "preserving" the span is really nice, rust analyzer will even figure out that
                    // the identifier in the macro is a nbt::Enum variant
                    let id = proc_macro2::Ident::new(&s, ident.span().into());
                    Ok(quote! {::nbt::Tag::#id})
                }
                _ => Err(syn::Error::new(ident.span().into(), ERR_MSG)),
            }
        }
        Some(other) => Err(syn::Error::new(other.span().into(), ERR_MSG)),
        None => Err(syn::Error::new(proc_macro2::Span::call_site(), ERR_MSG)),
    }
}

fn tagged_inner(
    args: TokenStream,
    input: TokenStream,
    derive_input: DeriveInput,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &derive_input.ident;

    match &derive_input.data {
        syn::Data::Struct(_) => (),
        _ => return Err(syn::Error::new(name.span(), "should be used on struct")),
    }

    let tag = to_tag(args)?;

    let (impl_generics, ty_generics, where_clause) = derive_input.generics.split_for_impl();

    let input_2 = proc_macro2::TokenStream::from(input);

    Ok(quote! {
        #input_2
        impl #impl_generics ::nbt::Tagged for #name #ty_generics #where_clause {
            const TAG: ::nbt::Tag = #tag;
        }
    })
}
