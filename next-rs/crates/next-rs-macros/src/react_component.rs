use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ItemFn};

use crate::util::{is_render_context, parameters, returns_result};

/// Expands `#[react_component(Component)]`.
pub fn expand(attr: TokenStream, function: ItemFn) -> syn::Result<TokenStream> {
    if attr.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.ident,
            "`#[react_component]` needs the registered component, e.g. \
             `#[react_component(Dashboard)]`",
        ));
    }
    let component: Ident = syn::parse2(attr)?;
    let signature = &function.sig;

    if signature.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            signature,
            "a `#[react_component]` loader must be `async`",
        ));
    }
    if !signature.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &signature.generics,
            "a `#[react_component]` loader cannot be generic: the loader manifest needs one \
             concrete signature",
        ));
    }
    if !returns_result(&signature.output) {
        return Err(syn::Error::new_spanned(
            &signature.output,
            "a `#[react_component]` loader must return `Result<Props>`",
        ));
    }

    let first = signature.inputs.first().ok_or_else(|| {
        syn::Error::new_spanned(
            signature,
            "a `#[react_component]` loader takes `ctx: RenderContext` first (spec §28)",
        )
    })?;
    match first {
        syn::FnArg::Typed(typed) if is_render_context(&typed.ty) => {}
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "the first parameter of a `#[react_component]` loader must be `RenderContext` \
                 (spec §28)",
            ));
        }
    }

    let loader_name = signature.ident.clone();
    let loader_id = loader_name.to_string();
    let visibility = &function.vis;
    let attributes = &function.attrs;

    // Every parameter after `RenderContext` is serialisable invocation state
    // (spec §29).
    let arguments = parameters(signature, 1)?;
    let arity = arguments.len();

    let impl_name = format_ident!("__next_rs_loader_impl_{}", loader_name);
    let registration_name = format_ident!("{}_loader", loader_name);

    let mut implementation = function.clone();
    implementation.sig.ident = impl_name.clone();
    implementation.vis = syn::Visibility::Inherited;
    implementation.attrs = Vec::new();

    let constructor_parameters = arguments.iter().map(|argument| {
        let name = &argument.name;
        let ty = &argument.ty;
        quote!(#name: #ty)
    });
    let constructor_args = arguments.iter().map(|argument| {
        let name = &argument.name;
        quote!(.with_arg(&#name))
    });

    let decodes = arguments.iter().enumerate().map(|(index, argument)| {
        let binding = &argument.name;
        let ty = &argument.ty;
        let label = binding.to_string();
        quote! {
            let #binding: #ty =
                ::next_rs::__private::core::decode_arg(&__args, #index, #label)?;
        }
    });
    let argument_names = arguments.iter().map(|argument| &argument.name);

    let constructor_doc = format!(
        "Builds a React slot for `{component}` backed by the `{loader_id}` loader.\n\nCalling \
         this does not run the loader (spec §27); the HTML transformer schedules it when it \
         reaches the slot's marker."
    );
    let registration_doc =
        format!("Registers the `{loader_id}` loader so `/__next_rs/react` can invoke it.");

    Ok(quote! {
        #implementation

        #(#attributes)*
        #[doc = #constructor_doc]
        #visibility fn #loader_name(
            #(#constructor_parameters),*
        ) -> ::next_rs::__private::react::ReactSlot {
            ::next_rs::__private::react::ReactSlot::new(#component::COMPONENT, #loader_id)
                #(#constructor_args)*
        }

        #[doc = #registration_doc]
        #visibility fn #registration_name(
        ) -> ::std::sync::Arc<dyn ::next_rs::__private::react::SlotLoader> {
            ::std::sync::Arc::new(::next_rs::__private::react::TypedLoader::new(
                #loader_id,
                #component::ID,
                #arity,
                |__ctx: ::next_rs::__private::react::RenderContext,
                 __args: ::std::vec::Vec<::next_rs::__private::serde_json::Value>| async move {
                    #(#decodes)*
                    let __props = #impl_name(__ctx, #(#argument_names),*).await?;
                    ::next_rs::__private::core::encode_result(__props)
                },
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use syn::parse_quote;

    use super::*;

    fn expand_str(function: ItemFn) -> String {
        expand(quote!(Dashboard), function)
            .unwrap()
            .to_token_stream()
            .to_string()
    }

    fn loader() -> ItemFn {
        parse_quote! {
            async fn dashboard(ctx: RenderContext, org_id: u64) -> Result<DashboardProps> {
                ctx.auth.require_access_to_org(org_id).await?;
                Ok(DashboardProps {})
            }
        }
    }

    #[test]
    fn generates_a_constructor_and_a_registration() {
        let expanded = expand_str(loader());
        // Public constructor, hidden executor (spec §27).
        assert!(expanded.contains(
            "fn dashboard (org_id : u64) -> :: next_rs :: __private :: react :: ReactSlot"
        ));
        assert!(expanded.contains("__next_rs_loader_impl_dashboard"));
        assert!(expanded.contains("fn dashboard_loader ()"));
        assert!(expanded.contains("Dashboard :: COMPONENT"));
        assert!(expanded.contains("Dashboard :: ID"));
        assert!(expanded.contains("\"dashboard\""));
        assert!(expanded.contains("with_arg (& org_id)"));
        assert!(expanded.contains("decode_arg"));
    }

    #[test]
    fn preserves_visibility() {
        let function: ItemFn = parse_quote! {
            pub async fn dashboard(ctx: RenderContext, org_id: u64) -> Result<P> { todo!() }
        };
        let expanded = expand_str(function);
        assert!(expanded.contains("pub fn dashboard (org_id : u64)"));
        assert!(expanded.contains("pub fn dashboard_loader"));
    }

    #[test]
    fn supports_several_serialisable_arguments() {
        // Spec §29.
        let function: ItemFn = parse_quote! {
            async fn project_card(
                ctx: RenderContext,
                org_id: OrgId,
                project_id: ProjectId,
                filters: Filters,
            ) -> Result<ProjectCardProps> { todo!() }
        };
        let expanded = expand(quote!(ProjectCard), function)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(expanded.contains("3usize") || expanded.contains("3 usize"));
        assert!(expanded.contains("with_arg (& org_id)"));
        assert!(expanded.contains("with_arg (& project_id)"));
        assert!(expanded.contains("with_arg (& filters)"));
    }

    #[test]
    fn supports_a_context_only_loader() {
        let function: ItemFn = parse_quote! {
            async fn banner(ctx: RenderContext) -> Result<BannerProps> { todo!() }
        };
        let expanded = expand(quote!(Banner), function)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(expanded.contains("fn banner () ->"));
        assert!(expanded.contains("0usize") || expanded.contains("0 usize"));
    }

    #[test]
    fn requires_a_component_name() {
        let error = expand(TokenStream::new(), loader()).unwrap_err();
        assert!(error.to_string().contains("needs the registered component"));
    }

    #[test]
    fn requires_async() {
        let function: ItemFn = parse_quote! {
            fn dashboard(ctx: RenderContext, org_id: u64) -> Result<P> { todo!() }
        };
        let error = expand(quote!(Dashboard), function).unwrap_err();
        assert!(error.to_string().contains("must be `async`"));
    }

    #[test]
    fn requires_a_result_return_type() {
        let function: ItemFn = parse_quote! {
            async fn dashboard(ctx: RenderContext, org_id: u64) -> DashboardProps { todo!() }
        };
        let error = expand(quote!(Dashboard), function).unwrap_err();
        assert!(error.to_string().contains("must return `Result<Props>`"));
    }

    #[test]
    fn requires_a_render_context_first() {
        let function: ItemFn = parse_quote! {
            async fn dashboard(org_id: u64) -> Result<P> { todo!() }
        };
        let error = expand(quote!(Dashboard), function).unwrap_err();
        assert!(error.to_string().contains("must be `RenderContext`"));

        let function: ItemFn = parse_quote! {
            async fn dashboard() -> Result<P> { todo!() }
        };
        let error = expand(quote!(Dashboard), function).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("takes `ctx: RenderContext` first")
        );
    }

    #[test]
    fn rejects_generic_loaders() {
        let function: ItemFn = parse_quote! {
            async fn dashboard<T>(ctx: RenderContext, value: T) -> Result<P> { todo!() }
        };
        let error = expand(quote!(Dashboard), function).unwrap_err();
        assert!(error.to_string().contains("cannot be generic"));
    }
}
