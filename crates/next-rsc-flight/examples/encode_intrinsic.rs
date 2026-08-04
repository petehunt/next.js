use next_rsc::{Node, element};

fn main() {
    let model = element(
        "main",
        [
            element("h1", [Node::text("Rust Flight")]),
            element("p", [Node::text("decoded by React")]),
        ],
    )
    .prop("data-flight", "rust");

    let bytes = next_rsc_flight::encode_root(&model).unwrap();
    print!("{}", String::from_utf8(bytes).unwrap());
}
