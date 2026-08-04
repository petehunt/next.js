use next_rsc_macros::page;
pub struct PageProps;
pub struct RequestContext;
pub struct Node;
pub struct RenderError;
pub type RenderResult = Result<Node, RenderError>;
#[page]
pub fn render(_props: PageProps, _ctx: RequestContext) -> RenderResult {
    Ok(Node)
}
fn main() {}
