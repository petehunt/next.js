use next_rsc::{Node, client_reference, element};

fn main() {
    let client = client_reference("42", "default", std::iter::empty::<&str>(), [])
        .prop("label", "Interactive from Rust")
        .prop("enabled", true);
    let model = element("main", [Node::text("server"), client]);
    let bytes = next_rsc_flight::encode_root(&model).unwrap();
    print!("{}", String::from_utf8(bytes).unwrap());
}
