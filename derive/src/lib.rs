//! Proc-macro helpers for `par-term-emu-core-rust` Python bindings (ARC-014).

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Expr, Fields, ItemStruct, LitStr, Path};

/// Attribute macro that adds `#[pyo3(get)]` to every named field of a struct.
///
/// Place it **above** `#[pyclass]` so PyO3 sees the generated getter attributes
/// (attribute macros expand top-down, so this runs first):
///
/// ```ignore
/// #[par_term_emu_derive::pyo3_get_all]
/// #[pyclass]
/// struct MyData { a: u32, b: String } // both become Python getters
/// ```
///
/// This removes the per-field `#[pyo3(get)]` boilerplate on the ~55 PyXxx data
/// classes (ARC-014). Tuple structs / unit structs are passed through unchanged.
#[proc_macro_attribute]
pub fn pyo3_get_all(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemStruct);

    if let syn::Fields::Named(named) = &mut input.fields {
        for field in named.named.iter_mut() {
            field.attrs.push(syn::parse_quote!(#[pyo3(get)]));
        }
    }

    quote! {
        #input
    }
    .into()
}

/// Derive macro generating the Python dict conversion for a protocol enum
/// (ARC-003): the dict shape produced by/consumed from the Python
/// `encode_*_message` / `decode_*_message` functions is derived from the enum
/// definition so it can never drift from the protocol, and every conversion is
/// exhaustive — a new variant fails compilation (or is picked up
/// automatically here) instead of being silently missed.
///
/// The generated inherent impl is `#[cfg(feature = "python")]`-gated and
/// provides:
///
/// - `py_type_tag(&self) -> &'static str` — the dict `"type"` value per
///   variant (snake_case of the variant name by default).
/// - `py_type_tags() -> &'static [&'static str]` — every tag, for error
///   messages.
/// - `to_py_dict(&self, py) -> PyResult<Bound<PyDict>>` — variant fields to
///   dict keys (field name == key name), `"type"` first.
/// - `from_py_kwargs(tag, kwargs) -> PyResult<Option<Self>>` — dict/kwargs to
///   variant; `Ok(None)` for an unknown tag.
///
/// Enums whose variants are all unit (e.g. `EventType`) only get the tag
/// methods plus `from_py_kwargs`.
///
/// # Attributes
///
/// On a variant:
///
/// - `#[pydict(type = "cursor")]` — custom `"type"` tag (e.g.
///   `CursorPosition` is `"cursor"`, not `"cursor_position"`).
/// - `#[pydict(to = "path")]` — delegate the whole variant's
///   `enum -> dict` conversion to `fn(py, &field1, &field2, ..) ->
///   PyResult<Bound<PyDict>>` (dict includes `"type"`). For shapes the
///   uniform field mapping cannot express (nested structs).
/// - `#[pydict(from = "path")]` — delegate the whole variant's
///   `kwargs -> enum` construction to `fn(Option<&Bound<PyDict>>) ->
///   PyResult<Self>`. For variants with constructor quirks that must be
///   preserved.
///
/// On a field:
///
/// - `#[pydict(to_with = "path")]` (alias: `with`) — custom field value ->
///   Python object: `fn(py, &FieldType) -> PyResult<Bound<PyAny>>`.
/// - `#[pydict(from_with = "path")]` — custom Python value -> field:
///   `fn(Option<&Bound<PyAny>>) -> FieldType` (applies its own default).
/// - `#[pydict(default = expr)]` — value used when the key is absent or has
///   the wrong type (mirrors the previous hand-written `unwrap_or` defaults).
/// - `#[pydict(encode_skip)]` — the Python encode API never reads this field;
///   it is constructed with `Default::default()` (e.g. `Output::timestamp`).
///
/// Missing/wrong-typed kwargs silently fall back to defaults, matching the
/// hand-written conversion this derive replaces.
#[proc_macro_derive(PyDictConvert, attributes(pydict))]
pub fn derive_py_dict_convert(input: TokenStream) -> TokenStream {
    expand_py_dict_convert(input.into()).into()
}

fn expand_py_dict_convert(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let input = match syn::parse2::<DeriveInput>(input) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error(),
    };
    let name = &input.ident;

    let variants = match &input.data {
        syn::Data::Enum(data) => &data.variants,
        _ => {
            return syn::Error::new_spanned(&input, "PyDictConvert only supports enums")
                .to_compile_error()
                .into();
        }
    };

    struct FieldInfo {
        ident: syn::Ident,
        ty: syn::Type,
        to_with: Option<Path>,
        from_with: Option<Path>,
        default: Option<Expr>,
        encode_skip: bool,
    }

    struct VariantInfo {
        ident: syn::Ident,
        tag: String,
        to_path: Option<Path>,
        from_path: Option<Path>,
        fields: Vec<FieldInfo>,
    }

    let mut infos = Vec::new();
    let mut errors: Vec<syn::Error> = Vec::new();

    for variant in variants {
        let mut tag = Some(snake_case(&variant.ident.to_string()));
        let mut to_path = None;
        let mut from_path = None;

        for attr in &variant.attrs {
            if !attr.path().is_ident("pydict") {
                continue;
            }
            if let Err(err) = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("type") {
                    let value = meta.value()?;
                    let lit: LitStr = value.parse()?;
                    tag = Some(lit.value());
                } else if meta.path.is_ident("to") {
                    to_path = Some(parse_path_value(&meta)?);
                } else if meta.path.is_ident("from") {
                    from_path = Some(parse_path_value(&meta)?);
                } else {
                    return Err(meta.error("unknown pydict variant attribute"));
                }
                Ok(())
            }) {
                errors.push(err);
            }
        }

        let mut fields = Vec::new();
        if let Fields::Named(named) = &variant.fields {
            for field in &named.named {
                let mut to_with = None;
                let mut from_with = None;
                let mut default = None;
                let mut encode_skip = false;
                for attr in &field.attrs {
                    if !attr.path().is_ident("pydict") {
                        continue;
                    }
                    if let Err(err) = attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("with") || meta.path.is_ident("to_with") {
                            to_with = Some(parse_path_value(&meta)?);
                        } else if meta.path.is_ident("from_with") {
                            from_with = Some(parse_path_value(&meta)?);
                        } else if meta.path.is_ident("default") {
                            let value = meta.value()?;
                            default = Some(value.parse::<Expr>()?);
                        } else if meta.path.is_ident("encode_skip") {
                            if meta.value().is_ok() {
                                return Err(meta.error("encode_skip takes no value"));
                            }
                            encode_skip = true;
                        } else {
                            return Err(meta.error("unknown pydict field attribute"));
                        }
                        Ok(())
                    }) {
                        errors.push(err);
                    }
                }
                fields.push(FieldInfo {
                    ident: field.ident.clone().expect("named field has ident"),
                    ty: field.ty.clone(),
                    to_with,
                    from_with,
                    default,
                    encode_skip,
                });
            }
        }

        infos.push(VariantInfo {
            ident: variant.ident.clone(),
            tag: tag.expect("tag always initialized"),
            to_path,
            from_path,
            fields,
        });
    }

    if !errors.is_empty() {
        let combined = errors
            .into_iter()
            .fold(None::<syn::Error>, |acc, err| match acc {
                Some(mut acc) => {
                    acc.combine(err);
                    Some(acc)
                }
                None => Some(err),
            })
            .expect("errors is non-empty");
        return combined.to_compile_error();
    }

    let all_unit = infos.iter().all(|info| info.fields.is_empty());

    // py_type_tag arms.
    let tag_arms = infos.iter().map(|info| {
        let ident = &info.ident;
        let tag = &info.tag;
        if info.fields.is_empty() {
            quote! { Self::#ident => #tag }
        } else {
            quote! { Self::#ident { .. } => #tag }
        }
    });

    // py_type_tags list.
    let tags = infos.iter().map(|info| info.tag.as_str());

    // to_py_dict arms.
    let to_arms = infos.iter().map(|info| {
        let ident = &info.ident;
        let tag = &info.tag;
        let to_path = &info.to_path;
        let field_idents = info.fields.iter().map(|f| &f.ident).collect::<Vec<_>>();
        if let Some(path) = to_path {
            if info.fields.is_empty() {
                quote! { Self::#ident => #path(py)? }
            } else {
                quote! { Self::#ident { #(#field_idents),* } => #path(py, #( #field_idents ),*)? }
            }
        } else {
            let set_type = quote! { __d.set_item("type", #tag)?; };
            let set_fields = info.fields.iter().map(|f| {
                let ident = &f.ident;
                let key = ident.to_string();
                match &f.to_with {
                    Some(path) => quote! { __d.set_item(#key, #path(py, #ident)?)?; },
                    None => quote! { __d.set_item(#key, #ident)?; },
                }
            });
            if info.fields.is_empty() {
                quote! {
                    Self::#ident => {
                        let __d = pyo3::types::PyDict::new(py);
                        #set_type
                        __d
                    }
                }
            } else {
                quote! {
                    Self::#ident { #( #field_idents ),* } => {
                        let __d = pyo3::types::PyDict::new(py);
                        #set_type
                        #( #set_fields )*
                        __d
                    }
                }
            }
        }
    });

    // from_py_kwargs arms.
    let from_arms = infos.iter().map(|info| {
        let ident = &info.ident;
        let tag = &info.tag;
        let from_path = &info.from_path;
        if let Some(path) = from_path {
            return quote! { #tag => #path(__kwargs)? };
        }
        if info.fields.is_empty() {
            return quote! { #tag => Self::#ident };
        }
        let inits = info.fields.iter().map(|f| {
            let ident = &f.ident;
            let key = ident.to_string();
            let ty = &f.ty;
            if f.encode_skip {
                quote! { #ident: ::core::default::Default::default() }
            } else if let Some(path) = &f.from_with {
                quote! { #ident: #path(__kv(#key).as_ref()) }
            } else if is_option_type(&f.ty) {
                quote! {
                    #ident: __kv(#key)
                        .and_then(|__v| __v.extract::<#ty>().ok())
                        .flatten()
                }
            } else if let Some(default) = &f.default {
                quote! {
                    #ident: __kv(#key)
                        .and_then(|__v| __v.extract::<#ty>().ok())
                        .unwrap_or(#default)
                }
            } else {
                quote! {
                    #ident: __kv(#key)
                        .and_then(|__v| __v.extract::<#ty>().ok())
                        .unwrap_or_default()
                }
            }
        });
        quote! {
            #tag => Self::#ident { #( #inits ),* }
        }
    });

    // Fieldless enums (EventType) get only the tag surface.
    let to_py_dict_method = if all_unit {
        quote! {}
    } else {
        quote! {
            /// Convert this message into its Python dict form: the `"type"`
            /// tag first, then one key per named field.
            pub fn to_py_dict<'py>(
                &self,
                py: pyo3::Python<'py>,
            ) -> pyo3::PyResult<pyo3::Bound<'py, pyo3::types::PyDict>> {
                let __dict = match self {
                    #( #to_arms ),*
                };
                Ok(__dict)
            }
        }
    };

    let expanded = quote! {
        #[cfg(feature = "python")]
        const _: () = {
            // Scope the trait imports the generated method bodies need,
            // independent of the target module's own imports.
            use pyo3::types::{PyAnyMethods, PyDictMethods};

            #[automatically_derived]
            impl #name {
            /// Python dict `"type"` tag for this variant (public Python API).
            pub fn py_type_tag(&self) -> &'static str {
                match self {
                    #( #tag_arms ),*
                }
            }

            /// Every valid Python dict `"type"` tag, in declaration order.
            pub fn py_type_tags() -> &'static [&'static str] {
                &[ #( #tags ),* ]
            }

            #to_py_dict_method

            /// Build a message from a `"type"` tag and its kwargs.
            ///
            /// Returns `Ok(None)` for an unknown tag; missing or
            /// wrong-typed kwargs silently fall back to per-field defaults.
            pub fn from_py_kwargs<'py>(
                __tag: &str,
                __kwargs: Option<&pyo3::Bound<'py, pyo3::types::PyDict>>,
            ) -> pyo3::PyResult<Option<Self>> {
                let __kv = |__key: &str| -> Option<pyo3::Bound<'py, pyo3::types::PyAny>> {
                    __kwargs.and_then(|__k| __k.get_item(__key).ok().flatten())
                };
                Ok(Some(match __tag {
                    #( #from_arms ),*
                    ,
                    _ => return Ok(None),
                }))
            }
        }
        };
    };

    expanded
}

fn parse_path_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Path> {
    let value = meta.value()?;
    let lit: LitStr = value.parse()?;
    lit.parse::<Path>()
}

fn snake_case(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn is_option_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(last) = type_path.path.segments.last() {
            return last.ident == "Option";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expansion_parses(src: &str) {
        let ts: proc_macro2::TokenStream = src.parse().unwrap();
        let out = expand_py_dict_convert(ts);
        syn::parse2::<syn::Item>(out).expect("expansion must be syntactically valid");
    }

    #[test]
    fn expansion_handles_fields_defaults_and_units() {
        expansion_parses(
            r#"
            enum Test {
                #[pydict(type = "a")]
                A {
                    #[pydict(default = 80)]
                    x: u16,
                    y: Option<String>,
                    #[pydict(encode_skip)]
                    z: Option<u64>,
                },
                B,
            }
        "#,
        );
    }

    #[test]
    fn expansion_handles_escape_hatch_paths() {
        expansion_parses(
            r#"
            enum Test {
                #[pydict(to = "some::to_fn", from = "some::from_fn")]
                A { x: u16 },
                #[pydict(type = "b")]
                B {
                    #[pydict(to_with = "f::to", from_with = "f::from")]
                    y: Vec<u8>,
                },
            }
        "#,
        );
    }

    #[test]
    fn expansion_fieldless_enum_gets_tags_only() {
        let ts: proc_macro2::TokenStream = "enum E { A, B }".parse().unwrap();
        let out = expand_py_dict_convert(ts).to_string();
        assert!(out.contains("py_type_tag"));
        assert!(out.contains("from_py_kwargs"));
        assert!(
            !out.contains("to_py_dict"),
            "fieldless enums need no dict form"
        );
    }
}
