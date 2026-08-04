//! Attribute exports for experimental Rust Server Components.

use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, ItemFn, ReturnType, Type, parse_macro_input, spanned::Spanned};

/// Marks a function as a Next.js Rust layout.
///
/// The native prototype calls the function by convention. Later compilation
/// targets can use this attribute to generate their exported ABI entrypoint.
#[proc_macro_attribute]
pub fn layout(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let attribute = proc_macro2::TokenStream::from(attribute);
    let function = parse_macro_input!(item as ItemFn);
    let mut errors: Option<syn::Error> = None;
    let mut report = |error: syn::Error| match &mut errors {
        Some(errors) => errors.combine(error),
        None => errors = Some(error),
    };

    if !attribute.is_empty() {
        report(syn::Error::new(
            attribute.span(),
            "#[next_rsc::layout] does not accept arguments",
        ));
    }
    if function.sig.ident != "render" {
        report(syn::Error::new(
            function.sig.ident.span(),
            "a Rust Server Component layout entrypoint must be named `render`",
        ));
    }
    if !matches!(&function.vis, syn::Visibility::Public(_)) {
        report(syn::Error::new(
            function.vis.span(),
            "a Rust Server Component layout entrypoint must be public",
        ));
    }
    if let Some(asyncness) = &function.sig.asyncness {
        report(syn::Error::new(
            asyncness.span(),
            "async layouts require the future host-call ABI and are not supported yet",
        ));
    }
    if !function.sig.generics.params.is_empty() || function.sig.generics.where_clause.is_some() {
        report(syn::Error::new(
            function.sig.generics.span(),
            "a layout entrypoint cannot be generic",
        ));
    }
    if function.sig.constness.is_some()
        || function.sig.unsafety.is_some()
        || function.sig.abi.is_some()
        || function.sig.variadic.is_some()
    {
        report(syn::Error::new(
            function.sig.span(),
            "a layout entrypoint must be a normal safe Rust function",
        ));
    }

    match function.sig.inputs.first() {
        Some(FnArg::Typed(argument)) if function.sig.inputs.len() == 1 => {
            if type_name(&argument.ty).as_deref() != Some("LayoutProps") {
                report(syn::Error::new(
                    argument.ty.span(),
                    "the layout argument must have type `LayoutProps`",
                ));
            }
        }
        _ => report(syn::Error::new(
            function.sig.inputs.span(),
            "a layout entrypoint must take exactly one `LayoutProps` argument",
        )),
    }

    match &function.sig.output {
        ReturnType::Type(_, output)
            if matches!(
                type_name(output).as_deref(),
                Some("RenderResult" | "Result")
            ) => {}
        _ => report(syn::Error::new(
            function.sig.output.span(),
            "a layout entrypoint must return `RenderResult` or `Result<Node, RenderError>`",
        )),
    }

    drop(report);
    let compile_errors = errors.map(|errors| errors.into_compile_error());
    quote!(#function #compile_errors).into()
}

/// Marks and validates a Rust page entrypoint. Synchronous pages take
/// `PageProps`; async pages additionally take a request-scoped `RequestContext`.
#[proc_macro_attribute]
pub fn page(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let attribute = proc_macro2::TokenStream::from(attribute);
    let function = parse_macro_input!(item as ItemFn);
    let mut errors: Option<syn::Error> = None;
    let mut report = |error: syn::Error| match &mut errors {
        Some(errors) => errors.combine(error),
        None => errors = Some(error),
    };
    if !attribute.is_empty() {
        report(syn::Error::new(
            attribute.span(),
            "#[next_rsc::page] does not accept arguments",
        ));
    }
    if function.sig.ident != "render" {
        report(syn::Error::new(
            function.sig.ident.span(),
            "a Rust Server Component page entrypoint must be named `render`",
        ));
    }
    if !matches!(&function.vis, syn::Visibility::Public(_)) {
        report(syn::Error::new(
            function.vis.span(),
            "a Rust Server Component page entrypoint must be public",
        ));
    }
    if !function.sig.generics.params.is_empty()
        || function.sig.generics.where_clause.is_some()
        || function.sig.constness.is_some()
        || function.sig.unsafety.is_some()
        || function.sig.abi.is_some()
        || function.sig.variadic.is_some()
    {
        report(syn::Error::new(
            function.sig.span(),
            "a page entrypoint must be a non-generic safe Rust function",
        ));
    }
    let expected: &[&str] = if function.sig.asyncness.is_some() {
        &["PageProps", "RequestContext"]
    } else {
        &["PageProps"]
    };
    let actual: Vec<_> = function
        .sig
        .inputs
        .iter()
        .filter_map(|argument| match argument {
            FnArg::Typed(argument) => type_name(&argument.ty),
            FnArg::Receiver(_) => None,
        })
        .collect();
    if actual != expected {
        report(syn::Error::new(
            function.sig.inputs.span(),
            if function.sig.asyncness.is_some() {
                "an async page must take exactly `PageProps, RequestContext`"
            } else {
                "a synchronous page must take exactly one `PageProps` argument"
            },
        ));
    }
    match &function.sig.output {
        ReturnType::Type(_, output)
            if matches!(
                type_name(output).as_deref(),
                Some("RenderResult" | "Result")
            ) => {}
        _ => report(syn::Error::new(
            function.sig.output.span(),
            "a page entrypoint must return `RenderResult` or `Result<Node, RenderError>`",
        )),
    }
    drop(report);
    let compile_errors = errors.map(|errors| errors.into_compile_error());
    quote!(#function #compile_errors).into()
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}
