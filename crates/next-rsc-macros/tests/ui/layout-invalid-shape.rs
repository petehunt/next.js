use next_rsc_macros::layout;

struct LayoutProps;

#[layout(unexpected)]
fn wrong_name<T>(_props: LayoutProps, _extra: T) {}

fn main() {}
