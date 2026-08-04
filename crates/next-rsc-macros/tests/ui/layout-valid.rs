use next_rsc_macros::layout;

struct LayoutProps;
struct Node;
struct RenderError;
type RenderResult = Result<Node, RenderError>;

#[layout]
pub fn render(_props: LayoutProps) -> RenderResult {
    Ok(Node)
}

fn main() {}
