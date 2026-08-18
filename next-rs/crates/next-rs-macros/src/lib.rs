//! Attribute macros for `next-rs` (spec §7, §10, §26).
//!
//! Generated code refers to `::next_rs::__private::*`, so a crate using these
//! macros depends on the `next-rs` facade rather than on every sub-crate.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

mod export;
mod react_component;
mod util;

/// Exposes a Rust function to TypeScript (spec §7).
///
/// ```ignore
/// #[export]
/// pub fn normalize_slug(value: String) -> String {
///     value.trim().to_lowercase().replace(' ', "-")
/// }
/// ```
///
/// ```ts
/// import { normalizeSlug } from "@app/rust"
/// ```
///
/// Exports are server-only unless marked `#[export(client)]`, which additionally
/// compiles them to browser WASM (spec §10, §11).
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    match export::expand(attr.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Binds a Rust props loader to a registered React Client Component (spec §26).
///
/// ```ignore
/// #[react_component(Dashboard)]
/// async fn dashboard(ctx: RenderContext, org_id: u64) -> Result<DashboardProps> {
///     ctx.auth.require_access_to_org(org_id).await?;
///     Ok(DashboardProps { /* ... */ })
/// }
/// ```
///
/// The annotation produces a public constructor `dashboard(org_id) -> ReactSlot`
/// and a hidden executor registered under the loader ID `dashboard`. Calling the
/// constructor does not run the loader (spec §27).
#[proc_macro_attribute]
pub fn react_component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    match react_component::expand(attr.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Declares a reference to a registered React Client Component (spec §25).
///
/// Emitted by `next-rs build` into the generated React bindings; also usable by
/// hand.
#[proc_macro]
pub fn define_component(item: TokenStream) -> TokenStream {
    let name = match syn::parse::<syn::Ident>(item) {
        Ok(name) => name,
        Err(error) => return error.to_compile_error().into(),
    };
    let id = name.to_string();
    let doc = format!("Reference to the registered React Client Component `{id}`.");
    quote! {
        #[doc = #doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct #name;

        impl #name {
            /// The component reference used to build slots.
            pub const COMPONENT: ::next_rs::__private::react::ComponentRef =
                ::next_rs::__private::react::ComponentRef::new(#id);
            /// The registered component ID.
            pub const ID: &'static str = #id;
        }
    }
    .into()
}
