use next_rsc_macros::page;
pub struct PageProps;
pub struct Node;
pub struct RenderError;
pub type RenderResult = Result<Node, RenderError>;
#[page]
pub fn render(_props: PageProps) -> RenderResult {
    Ok(Node)
}
fn main() {}
