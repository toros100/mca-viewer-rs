use syn::DeriveInput;
use syn::LifetimeParam;
use syn::spanned::Spanned;

pub struct ProcessedField {
    pub field: syn::Field,
    pub name_string: String, // field name or name determined by rename attr
    pub name_span: proc_macro2::Span, // span of where the name actually came from (field name or attr)
    pub optional: bool,
}

pub fn process_fields(
    input: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> syn::Result<Vec<ProcessedField>> {
    let mut pfs = Vec::<ProcessedField>::new();

    for f in input.iter() {
        if let Some(pf) = parse_field_attrs(f.clone())? {
            if let Some(other) = pfs.iter().find(|f| f.name_string == pf.name_string) {
                // showing the error on both duplicates, very fancy
                let mut err = syn::Error::new(pf.name_span, "duplicate name");
                err.combine(syn::Error::new(other.name_span, "duplicate name"));
                return Err(err);
            }

            if pf.name_string.len() > u16::MAX as usize {
                // surely someone will see this error
                return Err(syn::Error::new(
                    pf.name_span,
                    "name too long (max. 65535 bytes)",
                ));
            }

            pfs.push(pf);
        }
    }
    Ok(pfs)
}

pub fn parse_field_attrs(field: syn::Field) -> syn::Result<Option<ProcessedField>> {
    let mut rename: Option<(String, proc_macro2::Span)> = None;
    let mut skip = false;
    let mut optional = false;

    for attr in field.attrs.iter() {
        if attr.path().is_ident("nbt") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    // is this fine? does this break in ways i have not thought about? seems sketchy
                    if meta.input.peek(syn::Token![=]) {
                        return Err(meta.error("skip takes no value"));
                    }
                    skip = true
                } else if meta.path.is_ident("optional") {
                    if meta.input.peek(syn::Token![=]) {
                        return Err(meta.error("optional takes no value"));
                    }
                    optional = true
                } else if meta.path.is_ident("rename") {
                    if rename.is_some() {
                        return Err(meta.error("duplicate attribute \"rename\""));
                    }
                    let s = meta.value()?.parse::<syn::LitStr>()?;
                    _ = attr.span();
                    rename = Some((s.value(), s.span()));
                } else {
                    return Err(meta.error("unknown attribute"));
                }
                Ok(())
            })?;
        }
    }

    if skip {
        Ok(None)
    } else {
        let (name_string, name_span) = rename
            // inner unwrap is fine because we validated that all fields are named earlier
            .unwrap_or_else(|| (field.ident.clone().unwrap().to_string(), field.ident.span()));

        Ok(Some(ProcessedField {
            field,
            name_string,
            name_span,
            optional,
        }))
    }
}

#[derive(Default)]
pub struct StructAttributes {
    pub exit_early: bool,
}

// NOTE: only really applicable to derive(DeserializePayload)
pub fn parse_struct_attrs(attrs: &[syn::Attribute]) -> syn::Result<StructAttributes> {
    let mut ca = StructAttributes::default();
    for attr in attrs {
        if attr.path().is_ident("nbt") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("exit_early") {
                    // is this fine? does this break in ways i have not thought about? seems sketchy
                    if meta.input.peek(syn::Token![=]) {
                        return Err(meta.error("exit_early takes no value"));
                    }
                    ca.exit_early = true;
                } else {
                    return Err(meta.error("unknown attribute"));
                }
                Ok(())
            })?;
        }
    }
    Ok(ca)
}

pub fn get_borrow_lifetime(input: &DeriveInput) -> syn::Result<Option<&LifetimeParam>> {
    let mut lts = input.generics.lifetimes();

    match lts.next() {
        Some(lp) => match lts.next() {
            Some(_) => Err(syn::Error::new(
                lp.span(),
                "struct may have at most one lifetime parameter",
            )),
            None => Ok(Some(lp)),
        },
        None => Ok(None),
    }
}
