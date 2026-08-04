use next_rsc::{Node, PageProps, RenderError};

pub fn render(_props: PageProps) -> Result<Node, RenderError> {
    panic!("intentional Rust component panic")
}
