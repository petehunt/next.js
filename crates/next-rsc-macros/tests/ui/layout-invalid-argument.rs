use next_rsc_macros::layout;

struct PageProps;
struct Node;
struct RenderError;
type RenderResult = Result<Node, RenderError>;

#[layout]
pub fn render(_props: PageProps) -> RenderResult {
    Ok(Node)
}

fn main() {}
