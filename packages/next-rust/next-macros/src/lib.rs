//! Proc macros for `next-rs`.
//!
//! - `#[next::island]` + `#[next::slot(...)]` — server islands and their slot contracts
//! - `#[next::route(GET)]` — route handlers
//! - `#[derive(TsType)]` — the boundary IDL

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, ItemFn, LitStr, Type, parse::Parse, parse::ParseStream,
    parse_macro_input, punctuated::Punctuated, Token,
};

// ---------------------------------------------------------------------------
// #[next::island] / #[next::slot]
// ---------------------------------------------------------------------------

struct SlotAttr {
    name: LitStr,
    props: Type,
}

impl Parse for SlotAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut props = None;
        let pairs = Punctuated::<SlotArg, Token![,]>::parse_terminated(input)?;
        for pair in pairs {
            match pair {
                SlotArg::Name(v) => name = Some(v),
                SlotArg::Props(v) => props = Some(v),
            }
        }
        Ok(SlotAttr {
            name: name.ok_or_else(|| syn::Error::new(input.span(), "slot needs `name = \"...\"`"))?,
            props: props
                .ok_or_else(|| syn::Error::new(input.span(), "slot needs `props = Type`"))?,
        })
    }
}

enum SlotArg {
    Name(LitStr),
    Props(Type),
}

impl Parse for SlotArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        match key.to_string().as_str() {
            "name" => Ok(SlotArg::Name(input.parse()?)),
            "props" => Ok(SlotArg::Props(input.parse()?)),
            other => Err(syn::Error::new(key.span(), format!("unknown slot key `{other}`"))),
        }
    }
}

/// Declares a slot on an island. Purely a marker consumed by `#[island]`; it is
/// listed *after* `#[island]` in source order but attribute macros run outermost
/// first, so `#[island]` sees these in its own attribute list.
#[proc_macro_attribute]
pub fn slot(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // `#[island]` strips these. Reaching here means `#[slot]` was used alone.
    let item = proc_macro2::TokenStream::from(item);
    quote! {
        compile_error!("#[next::slot] must be applied to a function that also has #[next::island]");
        #item
    }
    .into()
}

/// Turns an async function into a registered server island.
///
/// Injects a `slot` closure bound to this render's nonce, which is how a
/// template built by `maud`/`askama`/`format!` can emit slots without any
/// task-local state — including from spawned child tasks.
#[proc_macro_attribute]
pub fn island(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut func = parse_macro_input!(item as ItemFn);
    let explicit_id = if attr.is_empty() {
        None
    } else {
        Some(parse_macro_input!(attr as LitStr))
    };

    // Pull `#[next::slot(...)]` / `#[slot(...)]` off the function.
    let mut slots = Vec::new();
    let mut errors = Vec::new();
    func.attrs.retain(|a| {
        let is_slot = a
            .path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "slot");
        if !is_slot {
            return true;
        }
        match a.parse_args::<SlotAttr>() {
            Ok(s) => slots.push(s),
            Err(e) => errors.push(e.to_compile_error()),
        }
        false
    });
    if !errors.is_empty() {
        return quote! { #(#errors)* }.into();
    }

    let fn_name = func.sig.ident.clone();
    let island_id = explicit_id
        .map(|l| l.value())
        .unwrap_or_else(|| fn_name.to_string());

    if func.sig.asyncness.is_none() {
        return syn::Error::new_spanned(&func.sig, "#[next::island] requires an async fn")
            .to_compile_error()
            .into();
    }

    // Props are the single argument, if any.
    let props_ty: Type = match func.sig.inputs.first() {
        Some(syn::FnArg::Typed(pat)) => (*pat.ty).clone(),
        Some(syn::FnArg::Receiver(r)) => {
            return syn::Error::new_spanned(r, "#[next::island] cannot take self")
                .to_compile_error()
                .into();
        }
        None => syn::parse_quote!(()),
    };
    let has_props = !func.sig.inputs.is_empty();

    let inner_name = format_ident!("__next_island_impl_{}", fn_name);
    let register_name = format_ident!("__next_island_register_{}", fn_name);

    let mut inner = func.clone();
    inner.sig.ident = inner_name.clone();
    inner.vis = syn::Visibility::Inherited;

    // `slot!` is injected as a `macro_rules!` rather than a closure or a
    // rewritten call. A closure would have to be generic over the props type,
    // and closures cannot be generic. Rewriting `slot(..)` call sites in the AST
    // fails for the case that matters most: `syn` keeps `format!`/`html!` bodies
    // as opaque token streams, and templates are exactly where slots are written.
    // A `macro_rules!` expands inside those bodies and resolves `__next_nonce`
    // at its definition site, so the per-render nonce is preserved without any
    // task-local — which is what lets an island that spawns still emit slots.
    let inner_block = inner.block.clone();
    inner.block = syn::parse_quote!({
        let __next_nonce = ::next_rs::fragment::new_nonce();

        #[allow(unused_macros)]
        macro_rules! slot {
            ($name:expr, $props:expr) => {
                ::next_rs::fragment::Slot::new(__next_nonce, $name, $props)
            };
        }

        let __next_out = async move #inner_block.await;
        (__next_nonce, __next_out)
    });
    // Rewrite the inner return type to the tuple.
    let orig_ret = match &func.sig.output {
        syn::ReturnType::Type(_, t) => (**t).clone(),
        syn::ReturnType::Default => syn::parse_quote!(()),
    };
    inner.sig.output = syn::parse_quote!(-> (u64, #orig_ret));

    let slot_names: Vec<_> = slots.iter().map(|s| &s.name).collect();
    let slot_props: Vec<_> = slots.iter().map(|s| &s.props).collect();

    let decode_props = if has_props {
        quote! {
            let props: #props_ty = match ::next_rs::deps::serde_json::from_value(__props) {
                Ok(p) => p,
                Err(e) => return Err(::next_rs::island::IslandError {
                    island: #island_id.to_owned(),
                    message: format!("invalid props: {e}"),
                }),
            };
            let (__nonce, __out) = #inner_name(props).await;
        }
    } else {
        quote! {
            let _ = __props;
            let (__nonce, __out) = #inner_name().await;
        }
    };

    let props_ts_expr = if has_props {
        quote! { <#props_ty as ::next_rs::ts::TsType>::ts_ref() }
    } else {
        quote! { "Record<string, never>".to_owned() }
    };
    let props_decls_expr = if has_props {
        quote! { <#props_ty as ::next_rs::ts::TsType>::ts_decls(out); }
    } else {
        quote! {}
    };

    quote! {
        #inner

        #[allow(non_snake_case)]
        mod #register_name {
            use super::*;

            fn render(
                __props: ::next_rs::deps::serde_json::Value,
            ) -> ::next_rs::island::BoxFuture<
                Result<::next_rs::fragment::Fragment, ::next_rs::island::IslandError>
            > {
                Box::pin(async move {
                    #decode_props
                    Ok(::next_rs::fragment::IntoFragment::into_fragment(__out, __nonce))
                })
            }

            fn props_ts() -> String { #props_ts_expr }

            fn decls(out: &mut ::std::collections::BTreeMap<String, ::next_rs::ts::TypeDecl>) {
                #props_decls_expr
                #(<#slot_props as ::next_rs::ts::TsType>::ts_decls(out);)*
            }

            static SLOTS: &[::next_rs::island::SlotDecl] = &[
                #(::next_rs::island::SlotDecl {
                    name: #slot_names,
                    props_ts: || <#slot_props as ::next_rs::ts::TsType>::ts_ref(),
                }),*
            ];

            ::next_rs::deps::inventory::submit! {
                ::next_rs::island::IslandDef {
                    id: #island_id,
                    slots: SLOTS,
                    props_ts,
                    decls,
                    render,
                }
            }
        }
    }
    .into()
}

// ---------------------------------------------------------------------------
// #[next::route(GET)]
// ---------------------------------------------------------------------------

/// Registers a Rust route handler.
///
/// The URL path is not written here: the build step derives it from the file's
/// location, exactly as it does for `route.ts`, and passes it through
/// `NEXT_ROUTE_PATH` at compile time.
#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);

    // `#[next::route(GET)]` or `#[next::route(GET, "/feeds/{format}")]`.
    // The build step normally supplies the path from the file's location, the
    // same way it does for `route.ts`; the explicit form exists for crates
    // compiled outside that step, and for the case the build cannot infer
    // (one crate, several routes) until per-route codegen lands.
    let raw = attr.to_string();
    let (method, explicit_path) = match raw.split_once(',') {
        None if raw.trim().is_empty() => ("GET".to_owned(), None),
        None => (raw.trim().to_uppercase(), None),
        Some((m, p)) => (
            m.trim().to_uppercase(),
            Some(p.trim().trim_matches('"').to_owned()),
        ),
    };

    let path_expr = match explicit_path {
        Some(p) => quote! { #p },
        None => quote! {
            match option_env!("NEXT_ROUTE_PATH") {
                Some(p) => p,
                None => concat!("/", module_path!()),
            }
        },
    };

    let fn_name = func.sig.ident.clone();
    let register_name = format_ident!("__next_route_register_{}", fn_name);

    quote! {
        #func

        #[allow(non_snake_case)]
        mod #register_name {
            use super::*;

            fn handler(
                req: ::next_rs::route::Request<::next_rs::route::Body>,
            ) -> ::next_rs::route::BoxFuture<
                Result<::next_rs::route::Response<::next_rs::route::Body>, ::next_rs::route::RouteError>
            > {
                Box::pin(async move { #fn_name(req).await })
            }

            ::next_rs::deps::inventory::submit! {
                ::next_rs::route::RouteDef {
                    path: #path_expr,
                    method: #method,
                    handler,
                }
            }
        }
    }
    .into()
}

// ---------------------------------------------------------------------------
// #[derive(TsType)]
// ---------------------------------------------------------------------------

/// Generates the TypeScript shape for a boundary type.
///
/// Only structs with named fields and C-like/newtype enums are supported. That
/// restriction is the point: slot props have to be statically declarable, and
/// arbitrary serde shapes are not.
#[proc_macro_derive(TsType, attributes(ts))]
pub fn derive_ts_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_str = name.to_string();
    // The emitted TypeScript has to match what serde actually puts on the wire.
    // If these disagree the generated `.d.ts` is worse than no types at all.
    let container_rename = serde_rename_all(&input.attrs);

    let (body_expr, dep_types): (proc_macro2::TokenStream, Vec<Type>) = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => {
                let mut parts = Vec::new();
                let mut deps = Vec::new();
                for f in &named.named {
                    let ident = f.ident.as_ref().unwrap();
                    let key = serde_rename(&f.attrs)
                        .unwrap_or_else(|| apply_rename(&ident.to_string(), container_rename.as_deref()));
                    let ty = &f.ty;
                    let optional = is_option(ty);
                    let q = if optional { "?" } else { "" };
                    parts.push(quote! {
                        format!("  {}{}: {}", #key, #q, <#ty as ::next_rs::ts::TsType>::ts_ref())
                    });
                    deps.push(ty.clone());
                }
                (
                    quote! {
                        {
                            let fields: Vec<String> = vec![#(#parts),*];
                            format!("{{\n{}\n}}", fields.join("\n"))
                        }
                    },
                    deps,
                )
            }
            Fields::Unnamed(un) if un.unnamed.len() == 1 => {
                let ty = &un.unnamed.first().unwrap().ty;
                (
                    quote! { <#ty as ::next_rs::ts::TsType>::ts_ref() },
                    vec![ty.clone()],
                )
            }
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "TsType supports structs with named fields or a single newtype field",
                )
                .to_compile_error()
                .into();
            }
        },
        Data::Enum(e) => {
            let mut variants = Vec::new();
            let mut deps = Vec::new();
            for v in &e.variants {
                let vname = v.ident.to_string();
                match &v.fields {
                    Fields::Unit => variants.push(quote! { format!("\"{}\"", #vname) }),
                    Fields::Unnamed(un) if un.unnamed.len() == 1 => {
                        let ty = &un.unnamed.first().unwrap().ty;
                        deps.push(ty.clone());
                        variants.push(quote! {
                            format!("{{ {}: {} }}", #vname, <#ty as ::next_rs::ts::TsType>::ts_ref())
                        });
                    }
                    _ => {
                        return syn::Error::new_spanned(
                            v,
                            "TsType enums support unit and single-field variants",
                        )
                        .to_compile_error()
                        .into();
                    }
                }
            }
            (
                quote! {
                    {
                        let vs: Vec<String> = vec![#(#variants),*];
                        vs.join(" | ")
                    }
                },
                deps,
            )
        }
        Data::Union(u) => {
            return syn::Error::new_spanned(u.union_token, "TsType does not support unions")
                .to_compile_error()
                .into();
        }
    };

    quote! {
        impl ::next_rs::ts::TsType for #name {
            fn ts_ref() -> String { #name_str.to_owned() }

            fn ts_decls(out: &mut ::std::collections::BTreeMap<String, ::next_rs::ts::TypeDecl>) {
                if out.contains_key(#name_str) {
                    return; // already emitted; also breaks recursive types
                }
                out.insert(#name_str.to_owned(), ::next_rs::ts::TypeDecl {
                    name: #name_str.to_owned(),
                    body: String::new(),
                });
                let body = #body_expr;
                out.insert(#name_str.to_owned(), ::next_rs::ts::TypeDecl {
                    name: #name_str.to_owned(),
                    body,
                });
                #(<#dep_types as ::next_rs::ts::TsType>::ts_decls(out);)*
            }
        }
    }
    .into()
}

fn is_option(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option"))
}

/// Reads `#[serde(rename_all = "...")]` off a container.
fn serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    serde_string_arg(attrs, "rename_all")
}

/// Reads `#[serde(rename = "...")]` off a field.
fn serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    serde_string_arg(attrs, "rename")
}

fn serde_string_arg(attrs: &[syn::Attribute], key: &str) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let mut found = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(key) {
                if let Ok(value) = meta.value() {
                    if let Ok(lit) = value.parse::<syn::LitStr>() {
                        found = Some(lit.value());
                    }
                }
            } else {
                // Consume any value so parsing continues past unrelated keys.
                let _ = meta.value().and_then(|v| v.parse::<syn::Expr>());
            }
            Ok(())
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Mirrors serde's `rename_all` transforms for the cases the IDL allows.
fn apply_rename(field: &str, rule: Option<&str>) -> String {
    match rule {
        Some("camelCase") => to_camel_case(field),
        Some("PascalCase") => {
            let c = to_camel_case(field);
            let mut out = String::with_capacity(c.len());
            let mut chars = c.chars();
            if let Some(f) = chars.next() {
                out.extend(f.to_uppercase());
            }
            out.push_str(chars.as_str());
            out
        }
        Some("kebab-case") => field.replace('_', "-"),
        Some("SCREAMING_SNAKE_CASE") => field.to_uppercase(),
        Some("lowercase") => field.to_lowercase(),
        // `snake_case`, unknown rules, and no rule all mean "as written".
        _ => field.to_owned(),
    }
}

fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}
