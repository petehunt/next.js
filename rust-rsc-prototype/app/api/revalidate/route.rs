use next_rsc::{MutationContext, RenderError, RevalidationPathKind};

pub fn handle(context: &mut MutationContext) -> Result<(), RenderError> {
    context.revalidate_tag("catalog", "max")?;
    context.revalidate_path("/catalog/rust", Some(RevalidationPathKind::Page))?;
    Ok(())
}
