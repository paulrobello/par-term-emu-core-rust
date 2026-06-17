//! Proc-macro helpers for `par-term-emu-core-rust` Python bindings (ARC-014).

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemStruct};

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
