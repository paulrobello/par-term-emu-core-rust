//! `VariantNames`: the variant-name registry of an enum (QA-003 follow-up).
//!
//! Generates `fn variant_name(&self) -> &'static str` (an exhaustive,
//! wildcard-free `match`, so the mapping can never drift from the enum) and
//! `const VARIANT_NAMES: &'static [&'static str]` (declaration order) as
//! `pub(crate)` inherent items. The name is the Rust variant identifier.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input: DeriveInput = match syn::parse2(input) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error(),
    };
    let syn::Data::Enum(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "VariantNames only supports enums")
            .to_compile_error();
    };

    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let names: Vec<String> = data.variants.iter().map(|v| v.ident.to_string()).collect();
    let arms = data.variants.iter().map(|v| {
        let variant = &v.ident;
        let name = variant.to_string();
        let pattern = match &v.fields {
            Fields::Unit => quote! { Self::#variant },
            Fields::Named(_) => quote! { Self::#variant { .. } },
            Fields::Unnamed(_) => quote! { Self::#variant(..) },
        };
        quote! { #pattern => #name, }
    });

    quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            /// Every variant name, in declaration order.
            pub(crate) const VARIANT_NAMES: &'static [&'static str] = &[#(#names),*];

            /// This value's variant name (`"Output"` for `Self::Output { .. }`).
            pub(crate) fn variant_name(&self) -> &'static str {
                match self {
                    #(#arms)*
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    use proc_macro2::TokenStream;

    fn run(src: &str) -> String {
        let out = expand(src.parse::<TokenStream>().unwrap());
        syn::parse_file(&out.to_string()).expect("expansion must be syntactically valid");
        out.to_string()
    }

    #[test]
    fn maps_unit_named_and_tuple_variants_exhaustively() {
        let out = run("enum E { A, B { x: u8 }, C(u8) }");
        assert!(out.contains("Self :: A => \"A\""));
        assert!(out.contains("Self :: B { .. } => \"B\""));
        assert!(out.contains("Self :: C (..) => \"C\""));
        assert!(out.contains("[\"A\" , \"B\" , \"C\"]"));
        assert!(!out.contains("_ =>"));
    }

    #[test]
    fn non_enum_is_a_compile_error() {
        assert!(run("struct S { a: u8 }").contains("compile_error"));
    }
}
