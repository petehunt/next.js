#[test]
fn validates_layout_signatures() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/layout-valid.rs");
    tests.compile_fail("tests/ui/layout-invalid-*.rs");
}
