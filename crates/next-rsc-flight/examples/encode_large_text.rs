fn main() {
    let model = next_rsc::Node::text("x".repeat(1024));
    let bytes = next_rsc_flight::encode_root(&model).unwrap();
    print!("{}", String::from_utf8(bytes).unwrap());
}
