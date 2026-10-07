//! `ProtoConvert`: app type <-> prost wire type conversions (ARC-006).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Fields, Ident, LitStr, Path};

/// Per-field conversion: the field's own type drives `ToWire`/`FromWire`
/// dispatch unless `with` names a module holding `to_wire`/`from_wire`.
struct FieldInfo {
    ident: Ident,
    ty: syn::Type,
    with: Option<Path>,
}

struct VariantInfo {
    ident: Ident,
    oneof_variant: Ident,
    message: Ident,
    fields: Vec<FieldInfo>,
    unit: bool,
}

#[derive(Default)]
struct ContainerAttrs {
    wire: Option<Path>,
    module: Option<Path>,
    oneof: Option<Path>,
    empty: Option<LitStr>,
}

fn parse_path(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Path> {
    let lit: LitStr = meta.value()?.parse()?;
    lit.parse::<Path>()
}

fn parse_ident(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Ident> {
    let lit: LitStr = meta.value()?.parse()?;
    lit.parse::<Ident>()
}

fn container_attrs(input: &DeriveInput) -> syn::Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("proto")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("wire") {
                out.wire = Some(parse_path(&meta)?);
            } else if meta.path.is_ident("module") {
                out.module = Some(parse_path(&meta)?);
            } else if meta.path.is_ident("oneof") {
                out.oneof = Some(parse_path(&meta)?);
            } else if meta.path.is_ident("empty") {
                out.empty = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unknown proto container attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn field_infos(fields: &Fields) -> syn::Result<Vec<FieldInfo>> {
    let mut out = Vec::new();
    let Fields::Named(named) = fields else {
        return Ok(out);
    };
    for field in &named.named {
        let mut with = None;
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("proto")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("with") {
                    with = Some(parse_path(&meta)?);
                    Ok(())
                } else {
                    Err(meta.error("unknown proto field attribute"))
                }
            })?;
        }
        out.push(FieldInfo {
            ident: field.ident.clone().expect("named field has ident"),
            ty: field.ty.clone(),
            with,
        });
    }
    Ok(out)
}

/// `app value -> wire value` for one field bound by reference as `#ident`.
fn to_wire_expr(f: &FieldInfo) -> TokenStream {
    let ident = &f.ident;
    let ty = &f.ty;
    match &f.with {
        Some(path) => quote! { #path::to_wire(#ident) },
        None => quote! {
            <#ty as crate::streaming::proto::ToWire<_>>::to_wire(#ident)
        },
    }
}

/// `wire value -> app value` for one field read from `#src.#ident`.
fn from_wire_expr(f: &FieldInfo, src: &Ident) -> TokenStream {
    let ident = &f.ident;
    let ty = &f.ty;
    match &f.with {
        Some(path) => quote! { #path::from_wire(#src.#ident)? },
        None => quote! {
            <#ty as crate::streaming::proto::FromWire<_>>::from_wire(#src.#ident)?
        },
    }
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = match syn::parse2::<DeriveInput>(input) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error(),
    };
    let attrs = match container_attrs(&input) {
        Ok(attrs) => attrs,
        Err(err) => return err.to_compile_error(),
    };
    let result = match &input.data {
        syn::Data::Enum(data) => expand_enum(&input.ident, &attrs, data),
        syn::Data::Struct(data) => expand_struct(&input.ident, &attrs, data),
        syn::Data::Union(_) => Err(syn::Error::new_spanned(
            &input,
            "ProtoConvert supports enums and structs",
        )),
    };
    result.unwrap_or_else(|err| err.to_compile_error())
}

/// A struct mapped field-for-field onto one prost message: `ToWire` and
/// `FromWire` for the type, its `Option`, and its `Vec`.
fn expand_struct(
    name: &Ident,
    attrs: &ContainerAttrs,
    data: &syn::DataStruct,
) -> syn::Result<TokenStream> {
    let wire = attrs.wire.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(
            name,
            "ProtoConvert on a struct needs #[proto(wire = \"...\")]",
        )
    })?;
    let fields = field_infos(&data.fields)?;
    let idents = fields.iter().map(|f| &f.ident).collect::<Vec<_>>();
    let to_fields = fields.iter().map(to_wire_expr);
    let src = format_ident!("__w");
    let from_fields = fields.iter().map(|f| from_wire_expr(f, &src));

    Ok(quote! {
        #[automatically_derived]
        impl crate::streaming::proto::ToWire<#wire> for #name {
            fn to_wire(&self) -> #wire {
                let Self { #( #idents ),* } = self;
                #wire { #( #idents: #to_fields ),* }
            }
        }

        #[automatically_derived]
        impl crate::streaming::proto::FromWire<#wire> for #name {
            fn from_wire(#src: #wire) -> crate::streaming::error::Result<Self> {
                Ok(Self { #( #idents: #from_fields ),* })
            }
        }

        #[automatically_derived]
        impl crate::streaming::proto::ToWire<::core::option::Option<#wire>>
            for ::core::option::Option<#name>
        {
            fn to_wire(&self) -> ::core::option::Option<#wire> {
                self.as_ref().map(crate::streaming::proto::ToWire::to_wire)
            }
        }

        #[automatically_derived]
        impl crate::streaming::proto::FromWire<::core::option::Option<#wire>>
            for ::core::option::Option<#name>
        {
            fn from_wire(
                w: ::core::option::Option<#wire>,
            ) -> crate::streaming::error::Result<Self> {
                w.map(<#name as crate::streaming::proto::FromWire<#wire>>::from_wire)
                    .transpose()
            }
        }

        #[automatically_derived]
        impl crate::streaming::proto::ToWire<::std::vec::Vec<#wire>> for ::std::vec::Vec<#name> {
            fn to_wire(&self) -> ::std::vec::Vec<#wire> {
                self.iter().map(crate::streaming::proto::ToWire::to_wire).collect()
            }
        }

        #[automatically_derived]
        impl crate::streaming::proto::FromWire<::std::vec::Vec<#wire>> for ::std::vec::Vec<#name> {
            fn from_wire(w: ::std::vec::Vec<#wire>) -> crate::streaming::error::Result<Self> {
                w.into_iter()
                    .map(<#name as crate::streaming::proto::FromWire<#wire>>::from_wire)
                    .collect()
            }
        }
    })
}

/// A message enum mapped onto a prost wrapper with a `message` oneof:
/// `From<&Enum> for Wire` and `TryFrom<Wire> for Enum`. Both matches are
/// exhaustive, so a variant added on either side fails to compile.
fn expand_enum(
    name: &Ident,
    attrs: &ContainerAttrs,
    data: &syn::DataEnum,
) -> syn::Result<TokenStream> {
    let missing = |what: &str| {
        syn::Error::new_spanned(
            name,
            format!("ProtoConvert on an enum needs #[proto({what} = \"...\")]"),
        )
    };
    let wire = attrs.wire.as_ref().ok_or_else(|| missing("wire"))?;
    let module = attrs.module.as_ref().ok_or_else(|| missing("module"))?;
    let oneof = attrs.oneof.as_ref().ok_or_else(|| missing("oneof"))?;
    let empty = attrs.empty.as_ref().ok_or_else(|| missing("empty"))?;

    let mut variants = Vec::new();
    for variant in &data.variants {
        let mut oneof_variant = variant.ident.clone();
        let mut message = variant.ident.clone();
        for attr in variant.attrs.iter().filter(|a| a.path().is_ident("proto")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("oneof_variant") {
                    oneof_variant = parse_ident(&meta)?;
                } else if meta.path.is_ident("message") {
                    message = parse_ident(&meta)?;
                } else {
                    return Err(meta.error("unknown proto variant attribute"));
                }
                Ok(())
            })?;
        }
        variants.push(VariantInfo {
            ident: variant.ident.clone(),
            oneof_variant,
            message,
            fields: field_infos(&variant.fields)?,
            unit: matches!(variant.fields, Fields::Unit),
        });
    }

    let src = format_ident!("__w");
    let to_arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let oneof_variant = &v.oneof_variant;
        let message = &v.message;
        let idents = v.fields.iter().map(|f| &f.ident).collect::<Vec<_>>();
        let values = v.fields.iter().map(to_wire_expr);
        let pattern = if v.unit {
            quote! { #name::#ident }
        } else {
            quote! { #name::#ident { #( #idents ),* } }
        };
        quote! {
            #pattern => #oneof::#oneof_variant(#module::#message { #( #idents: #values ),* })
        }
    });

    let from_arms = variants.iter().map(|v| {
        let ident = &v.ident;
        let oneof_variant = &v.oneof_variant;
        if v.unit {
            return quote! { Some(#oneof::#oneof_variant(_)) => Ok(#name::#ident) };
        }
        let idents = v.fields.iter().map(|f| &f.ident);
        let values = v.fields.iter().map(|f| from_wire_expr(f, &src));
        quote! {
            Some(#oneof::#oneof_variant(#src)) => Ok(#name::#ident { #( #idents: #values ),* })
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl ::core::convert::From<&#name> for #wire {
            fn from(msg: &#name) -> Self {
                let message = match msg {
                    #( #to_arms ),*
                };
                #wire { message: Some(message) }
            }
        }

        #[automatically_derived]
        impl ::core::convert::TryFrom<#wire> for #name {
            type Error = crate::streaming::error::StreamingError;

            fn try_from(msg: #wire) -> crate::streaming::error::Result<Self> {
                match msg.message {
                    #( #from_arms, )*
                    None => Err(crate::streaming::error::StreamingError::InvalidMessage(
                        #empty.into(),
                    )),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::expand;

    fn expansion(src: &str) -> String {
        let ts: TokenStream = src.parse().unwrap();
        let out = expand(ts);
        syn::parse_file(&out.to_string()).expect("expansion must be syntactically valid");
        out.to_string()
    }

    use proc_macro2::TokenStream;

    #[test]
    fn enum_expansion_covers_renames_units_and_with() {
        let out = expansion(
            r#"
            #[proto(wire = "pb::Msg", module = "pb", oneof = "pb::msg::Message", empty = "Empty msg")]
            enum E {
                #[proto(oneof_variant = "Cur", message = "CursorPos")]
                Cursor { col: u16 },
                Bell,
                Pct { #[proto(with = "pct")] percent: Option<u8> },
            }
            "#,
        );
        assert!(out.contains("Cur"));
        assert!(out.contains("CursorPos"));
        assert!(out.contains("pct :: to_wire"));
        assert!(out.contains("Empty msg"));
    }

    #[test]
    fn struct_expansion_emits_option_and_vec_impls() {
        let out = expansion(
            r#"
            #[proto(wire = "pb::Stats")]
            struct Stats { a: u64, b: Option<String> }
            "#,
        );
        assert!(out.contains("Option < pb :: Stats >"));
        assert!(out.contains("Vec < pb :: Stats >"));
    }

    #[test]
    fn enum_without_wire_attrs_is_a_compile_error() {
        let ts: TokenStream = "enum E { A }".parse().unwrap();
        assert!(expand(ts).to_string().contains("compile_error"));
    }
}
