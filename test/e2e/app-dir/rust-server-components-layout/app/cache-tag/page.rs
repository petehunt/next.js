use std::sync::atomic::{AtomicU32, Ordering};

use next_rsc::{Node, PageProps, RenderError, element};

static RENDERS: AtomicU32 = AtomicU32::new(0);

pub fn render(props: PageProps) -> Result<Node, RenderError> {
    let render = RENDERS.fetch_add(1, Ordering::Relaxed) + 1;
    props.request.cache_tag("rust-products")?;
    Ok(element(
        "p",
        [Node::text(format!("Rust cache tag render {render}"))],
    ))
}
