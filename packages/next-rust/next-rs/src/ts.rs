//! The type-mapping IDL.
//!
//! Every type that crosses the boundary — client call arguments, server call
//! returns, route handler bodies, slot props — declares its TypeScript shape
//! through [`TsType`]. The build step walks the registry and emits `.d.ts`.
//!
//! This is deliberately narrower than "anything serde can serialize". Slot props
//! must be *statically declarable*: if a prop type could only be described as
//! `unknown`, then `<Island of={Chart}><Island of={Legend} slot="legend" /></Island>`
//! loses the type checking that makes slots worth using. So the IDL is a closed
//! set of shapes with a derive macro, not an open reflection mechanism.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A named TypeScript declaration emitted into the generated `.d.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub body: String,
}

/// Types that can cross the Rust/TypeScript boundary.
pub trait TsType {
    /// How the type is referred to in a signature (`string`, `Foo`, `Foo[]`).
    fn ts_ref() -> String;

    /// Named declarations this type contributes, including its dependencies'.
    /// Collected into a set, so emitting the same struct twice is harmless.
    fn ts_decls(_out: &mut BTreeMap<String, TypeDecl>) {}
}

macro_rules! primitive {
    ($($rust:ty => $ts:literal),* $(,)?) => {
        $(impl TsType for $rust {
            fn ts_ref() -> String { $ts.to_owned() }
        })*
    };
}

primitive! {
    i8 => "number", i16 => "number", i32 => "number", i64 => "number", isize => "number",
    u8 => "number", u16 => "number", u32 => "number", u64 => "number", usize => "number",
    f32 => "number", f64 => "number",
    bool => "boolean",
    String => "string", &str => "string",
    () => "void",
    serde_json::Value => "unknown",
}

impl<T: TsType> TsType for Vec<T> {
    fn ts_ref() -> String {
        format!("{}[]", wrap(&T::ts_ref()))
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        T::ts_decls(out);
    }
}

impl<T: TsType> TsType for Option<T> {
    fn ts_ref() -> String {
        format!("{} | null", T::ts_ref())
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        T::ts_decls(out);
    }
}

impl<T: TsType> TsType for BTreeSet<T> {
    fn ts_ref() -> String {
        format!("{}[]", wrap(&T::ts_ref()))
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        T::ts_decls(out);
    }
}

impl<V: TsType> TsType for HashMap<String, V> {
    fn ts_ref() -> String {
        format!("Record<string, {}>", V::ts_ref())
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        V::ts_decls(out);
    }
}

impl<V: TsType> TsType for BTreeMap<String, V> {
    fn ts_ref() -> String {
        format!("Record<string, {}>", V::ts_ref())
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        V::ts_decls(out);
    }
}

impl<T: TsType, E> TsType for Result<T, E> {
    fn ts_ref() -> String {
        T::ts_ref()
    }
    fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
        T::ts_decls(out);
    }
}

macro_rules! tuple {
    ($($name:ident),+) => {
        impl<$($name: TsType),+> TsType for ($($name,)+) {
            fn ts_ref() -> String {
                let parts: Vec<String> = vec![$($name::ts_ref()),+];
                format!("[{}]", parts.join(", "))
            }
            fn ts_decls(out: &mut BTreeMap<String, TypeDecl>) {
                $($name::ts_decls(out);)+
            }
        }
    };
}

tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);

/// Parenthesizes a union before suffixing `[]`, so `Option<T>` inside a `Vec`
/// renders as `(T | null)[]` rather than the wrong `T | null[]`.
fn wrap(ts: &str) -> String {
    if ts.contains('|') && !ts.starts_with('(') {
        format!("({ts})")
    } else {
        ts.to_owned()
    }
}

/// Renders a declaration set as a `.d.ts` body, sorted for stable output so a
/// rebuild that changed nothing produces a byte-identical file.
pub fn render_decls(decls: &BTreeMap<String, TypeDecl>) -> String {
    let mut out = String::new();
    for decl in decls.values() {
        out.push_str(&format!("export type {} = {}\n\n", decl.name, decl.body));
    }
    out
}
