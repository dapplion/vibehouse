use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, punctuated::Punctuated, DeriveInput, Expr, ExprLit, GenericParam,
    LifetimeParam, Lit, Meta, Token, WhereClause,
};

#[proc_macro_attribute]
pub fn context_deserialize(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr with Punctuated::<Meta, Token![,]>::parse_terminated);
    let input = parse_macro_input!(item as DeriveInput);
    let ident = &input.ident;

    let mut ctx_types = Vec::new();
    let mut explicit_where: Option<WhereClause> = None;

    for meta in &args {
        match meta {
            Meta::Path(p) => {
                ctx_types.push(p.clone());
            }
            Meta::NameValue(nv) if nv.path.is_ident("bound") => {
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(lit_str),
                    ..
                }) = &nv.value
                {
                    let where_string = format!("where {}", lit_str.value());
                    match syn::parse_str::<WhereClause>(&where_string) {
                        Ok(where_clause) => {
                            explicit_where = Some(where_clause);
                        }
                        Err(err) => {
                            return syn::Error::new_spanned(
                                lit_str,
                                format!("Invalid where clause '{}': {}", lit_str.value(), err),
                            )
                            .to_compile_error()
                            .into();
                        }
                    }
                } else {
                    return syn::Error::new_spanned(
                        nv,
                        "Expected a string literal for `bound` value",
                    )
                    .to_compile_error()
                    .into();
                }
            }
            other => {
                return syn::Error::new_spanned(
                    other,
                    "Expected paths or `bound = \"...\"` in #[context_deserialize(...)]",
                )
                .to_compile_error()
                .into();
            }
        }
    }

    if ctx_types.is_empty() {
        return quote! {
            compile_error!("Usage: #[context_deserialize(Type1, Type2, ..., bound = \"...\")]");
        }
        .into();
    }

    let original_generics = input.generics.clone();

    // Clone and clean generics for impl use (remove default params)
    let mut impl_generics = input.generics.clone();
    for param in &mut impl_generics.params {
        if let GenericParam::Type(ty) = param {
            ty.eq_token = None;
            ty.default = None;
        }
    }

    // Ensure 'de lifetime exists in impl generics
    let has_de = impl_generics
        .lifetimes()
        .any(|LifetimeParam { lifetime, .. }| lifetime.ident == "de");

    if !has_de {
        impl_generics.params.insert(0, syn::parse_quote! { 'de });
    }

    let (_, ty_generics, _) = original_generics.split_for_impl();
    let (impl_gens, _, _) = impl_generics.split_for_impl();

    // Generate: no `'de` applied to the type name
    let mut impls = quote! {};
    for ctx in ctx_types {
        impls.extend(quote! {
            impl #impl_gens context_deserialize::ContextDeserialize<'de, #ctx>
                for #ident #ty_generics
                #explicit_where
            {
                fn context_deserialize<D>(
                    deserializer: D,
                    _context: #ctx,
                ) -> Result<Self, D::Error>
                where
                    D: serde::de::Deserializer<'de>,
                {
                    <Self as serde::Deserialize>::deserialize(deserializer)
                }
            }
        });
    }

    quote! {
        #input
        #impls
    }
    .into()
}
