use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ItemFn};

use crate::util::{parameters, returns_result, to_camel_case};

/// Expands `#[export]` / `#[export(client)]`.
pub fn expand(attr: TokenStream, function: ItemFn) -> syn::Result<TokenStream> {
    let target = parse_target(attr)?;
    let signature = &function.sig;

    if signature.constness.is_some() {
        return Err(syn::Error::new_spanned(
            signature,
            "a `const fn` cannot be exported; remove `const` or wrap it",
        ));
    }
    if !signature.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &signature.generics,
            "a generic function cannot be exported: TypeScript bindings need one concrete \
             signature",
        ));
    }

    let name = signature.ident.clone();
    let name_string = name.to_string();
    let js_name = to_camel_case(&name_string);
    let parameters = parameters(signature, 0)?;
    let arity = parameters.len();
    let is_async = signature.asyncness.is_some();
    let uses_result = returns_result(&signature.output);

    let decodes = parameters.iter().enumerate().map(|(index, parameter)| {
        let binding = &parameter.name;
        let ty = &parameter.ty;
        let label = binding.to_string();
        quote! {
            let #binding: #ty =
                ::next_rs::__private::core::decode_arg(&__args, #index, #label)?;
        }
    });
    let argument_names = parameters.iter().map(|parameter| &parameter.name);

    let call = {
        let awaited = if is_async {
            quote!(#name( #(#argument_names),* ).await)
        } else {
            quote!(#name( #(#argument_names),* ))
        };
        if uses_result {
            quote!(#awaited?)
        } else {
            quote!(#awaited)
        }
    };

    let registration_fn = format_ident!("__next_rs_export_{}", name);
    let target_tokens = match target {
        Target::Server => quote!(::next_rs::__private::core::ExportTarget::Server),
        Target::Client => quote!(::next_rs::__private::core::ExportTarget::Client),
    };
    let doc = format!(
        "Registration for the `#[export]` function [`{name_string}`], imported from TypeScript as \
         `{js_name}`."
    );

    Ok(quote! {
        #function

        #[doc = #doc]
        #[doc(hidden)]
        pub fn #registration_fn() -> ::next_rs::__private::core::ExportRegistration {
            ::next_rs::__private::core::ExportRegistration::new(
                #name_string,
                #js_name,
                #arity,
                #is_async,
                #target_tokens,
                ::std::sync::Arc::new(
                    |__args: ::std::vec::Vec<::next_rs::__private::serde_json::Value>| {
                        ::std::boxed::Box::pin(async move {
                            #(#decodes)*
                            let __result = #call;
                            ::next_rs::__private::core::encode_result(__result)
                        })
                    },
                ),
            )
        }
    })
}

enum Target {
    Server,
    Client,
}

/// Parses the attribute argument: nothing, or `client`.
fn parse_target(attr: TokenStream) -> syn::Result<Target> {
    if attr.is_empty() {
        return Ok(Target::Server);
    }
    let ident: Ident = syn::parse2(attr)?;
    if ident == "client" {
        Ok(Target::Client)
    } else {
        Err(syn::Error::new_spanned(
            &ident,
            "expected `#[export]` or `#[export(client)]`",
        ))
    }
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use syn::parse_quote;

    use super::*;

    fn expand_str(attr: TokenStream, function: ItemFn) -> String {
        expand(attr, function)
            .unwrap()
            .to_token_stream()
            .to_string()
    }

    #[test]
    fn generates_a_registration_for_a_sync_export() {
        let function: ItemFn = parse_quote! {
            pub fn normalize_slug(value: String) -> String { value }
        };
        let expanded = expand_str(TokenStream::new(), function);
        assert!(expanded.contains("__next_rs_export_normalize_slug"));
        assert!(expanded.contains("\"normalize_slug\""));
        assert!(expanded.contains("\"normalizeSlug\""));
        assert!(expanded.contains("ExportTarget :: Server"));
        assert!(expanded.contains("decode_arg"));
        // No `?` for a plain return type, and no `.await` for a sync function.
        assert!(!expanded.contains("normalize_slug (value) . await"));
    }

    #[test]
    fn awaits_async_exports_and_propagates_results() {
        let function: ItemFn = parse_quote! {
            pub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> { todo!() }
        };
        let expanded = expand_str(TokenStream::new(), function);
        assert!(expanded.contains("search (input) . await ?"));
        assert!(expanded.contains("true"), "is_async should be recorded");
    }

    #[test]
    fn client_exports_are_marked() {
        let function: ItemFn = parse_quote! {
            pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> { todo!() }
        };
        let expanded = expand_str(quote!(client), function);
        assert!(expanded.contains("ExportTarget :: Client"));
        assert!(expanded.contains("2usize") || expanded.contains("2 usize"));
    }

    #[test]
    fn keeps_the_original_function() {
        let function: ItemFn = parse_quote! {
            pub fn normalize_slug(value: String) -> String { value }
        };
        let expanded = expand_str(TokenStream::new(), function);
        assert!(expanded.contains("pub fn normalize_slug (value : String) -> String"));
    }

    #[test]
    fn zero_argument_exports_work() {
        let function: ItemFn = parse_quote! { pub fn version() -> String { todo!() } };
        let expanded = expand_str(TokenStream::new(), function);
        assert!(expanded.contains("0usize") || expanded.contains("0 usize"));
        assert!(!expanded.contains("decode_arg"));
    }

    #[test]
    fn rejects_an_unknown_attribute() {
        let function: ItemFn = parse_quote! { pub fn f() {} };
        let error = expand(quote!(server), function).unwrap_err();
        assert!(error.to_string().contains("expected `#[export]`"));
    }

    #[test]
    fn rejects_generic_and_const_functions() {
        let function: ItemFn = parse_quote! { pub fn f<T>(value: T) -> T { value } };
        assert!(
            expand(TokenStream::new(), function)
                .unwrap_err()
                .to_string()
                .contains("generic")
        );

        let function: ItemFn = parse_quote! { pub const fn f() -> u8 { 1 } };
        assert!(
            expand(TokenStream::new(), function)
                .unwrap_err()
                .to_string()
                .contains("const fn")
        );
    }

    #[test]
    fn rejects_methods_and_destructured_arguments() {
        let function: ItemFn = parse_quote! { pub fn f(&self) {} };
        assert!(expand(TokenStream::new(), function).is_err());

        let function: ItemFn = parse_quote! { pub fn f((a, b): (u8, u8)) {} };
        assert!(expand(TokenStream::new(), function).is_err());
    }
}
