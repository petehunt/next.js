use syn::{FnArg, Ident, Pat, ReturnType, Signature, Type};

/// One typed parameter of an annotated function.
pub struct Parameter {
    pub name: Ident,
    pub ty: Type,
}

/// Extracts the typed parameters of `signature`, skipping the first `skip`.
///
/// `self` receivers and destructuring patterns are rejected: a generated shim
/// has to name each argument, and a silently-renamed argument would produce
/// confusing errors in the generated TypeScript.
pub fn parameters(signature: &Signature, skip: usize) -> syn::Result<Vec<Parameter>> {
    let mut parameters = Vec::new();
    for input in signature.inputs.iter().skip(skip) {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "next-rs cannot export a method that takes `self`",
                ));
            }
            FnArg::Typed(typed) => {
                let Pat::Ident(pattern) = &*typed.pat else {
                    return Err(syn::Error::new_spanned(
                        &typed.pat,
                        "next-rs needs a plain identifier here so it can name the argument",
                    ));
                };
                parameters.push(Parameter {
                    name: pattern.ident.clone(),
                    ty: (*typed.ty).clone(),
                });
            }
        }
    }
    Ok(parameters)
}

/// True when the return type's outermost path segment is `Result`.
///
/// Used to decide whether the generated shim applies `?`.
pub fn returns_result(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    let Type::Path(path) = &**ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Result")
}

/// True when `ty` names `RenderContext`, however it is qualified.
pub fn is_render_context(ty: &Type) -> bool {
    let ty = match ty {
        // `ctx: &RenderContext` is accepted for readability even though the spec
        // writes it by value.
        Type::Reference(reference) => &*reference.elem,
        other => other,
    };
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "RenderContext")
}

/// Converts a Rust `snake_case` name to the TypeScript `camelCase` form.
pub fn to_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = false;
    for (index, character) in name.chars().enumerate() {
        if character == '_' {
            // A leading underscore is preserved so `_internal` does not become
            // `internal`.
            if index == 0 {
                out.push('_');
            } else {
                upper_next = true;
            }
            continue;
        }
        if upper_next {
            out.extend(character.to_uppercase());
            upper_next = false;
        } else {
            out.push(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn converts_names_to_camel_case() {
        assert_eq!(to_camel_case("normalize_slug"), "normalizeSlug");
        assert_eq!(to_camel_case("search"), "search");
        assert_eq!(to_camel_case("fuzzy_search_all"), "fuzzySearchAll");
        assert_eq!(to_camel_case("_internal_helper"), "_internalHelper");
        assert_eq!(to_camel_case("a_b_c"), "aBC");
        assert_eq!(to_camel_case(""), "");
        // Trailing underscores have nothing to capitalise.
        assert_eq!(to_camel_case("trailing_"), "trailing");
    }

    #[test]
    fn detects_result_returns() {
        let function: syn::ItemFn = parse_quote! { fn f() -> Result<u8> { todo!() } };
        assert!(returns_result(&function.sig.output));

        let function: syn::ItemFn = parse_quote! { fn f() -> next_rs::Result<u8> { todo!() } };
        assert!(returns_result(&function.sig.output));

        let function: syn::ItemFn = parse_quote! { fn f() -> String { todo!() } };
        assert!(!returns_result(&function.sig.output));

        let function: syn::ItemFn = parse_quote! { fn f() {} };
        assert!(!returns_result(&function.sig.output));

        let function: syn::ItemFn = parse_quote! { fn f() -> (u8, u8) { todo!() } };
        assert!(!returns_result(&function.sig.output));
    }

    #[test]
    fn detects_the_render_context_parameter() {
        assert!(is_render_context(&parse_quote!(RenderContext)));
        assert!(is_render_context(&parse_quote!(next_rs::RenderContext)));
        assert!(is_render_context(&parse_quote!(&RenderContext)));
        assert!(!is_render_context(&parse_quote!(u64)));
        assert!(!is_render_context(&parse_quote!(Context)));
    }

    #[test]
    fn extracts_parameters_after_the_skipped_ones() {
        let function: syn::ItemFn = parse_quote! {
            async fn dashboard(ctx: RenderContext, org_id: u64, filters: Filters) -> Result<P> {
                todo!()
            }
        };
        let parameters = parameters(&function.sig, 1).unwrap();
        assert_eq!(parameters.len(), 2);
        assert_eq!(parameters[0].name.to_string(), "org_id");
        assert_eq!(parameters[1].name.to_string(), "filters");
    }

    #[test]
    fn rejects_self_receivers_and_patterns() {
        let function: syn::ItemFn = parse_quote! {
            fn f(&self, a: u8) -> u8 { todo!() }
        };
        assert!(parameters(&function.sig, 0).is_err());

        let function: syn::ItemFn = parse_quote! {
            fn f((a, b): (u8, u8)) -> u8 { todo!() }
        };
        assert!(parameters(&function.sig, 0).is_err());
    }
}
