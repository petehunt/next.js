#[test]
fn validates_page_signatures() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/page-valid-*.rs");
    tests.compile_fail("tests/ui/page-invalid-*.rs");
}
