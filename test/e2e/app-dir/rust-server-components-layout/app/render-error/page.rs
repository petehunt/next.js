use next_rsc::{Node, PageProps, RenderError};

pub fn render(_props: PageProps) -> Result<Node, RenderError> {
    Err(RenderError::new("intentional returned Rust render error"))
}
