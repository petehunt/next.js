# `operations` — the example application

The tree from spec §6, with the Rust-owned HTML document from §95.

```text
operations/
├── app/
│   ├── proxy.rs              Rust request preprocessing (§13)
│   ├── page.tsx              Next owns /
│   ├── api/
│   │   ├── users/route.rs    Rust owns /api/users (§17)
│   │   ├── billing/route.ts  Next owns /api/billing
│   │   └── internal/route.rs A mounted Axum router owns /api/internal/* (§20)
│   └── operations/route.rs   Rust owns the HTML document (§22, §95)
├── components/               Client Components referenced from Rust (§24)
├── rust/src/
│   ├── lib.rs                #[export] and #[export(client)] (§7, §10)
│   └── react.rs              #[react_component] loaders (§26)
└── next-rs.components.ts     The component registry (§24)
```

Ownership:

| URL | Owner |
|---|---|
| `/` | Next |
| `/api/users` | Rust |
| `/api/billing` | Next |
| `/api/internal/*` | Rust (mounted Axum router) |
| `/operations` | Rust HTML with React slots |

This directory is a fixture, not a workspace member: `packages/next-rs`'
`example.test.ts` runs the real build against it, so the manifests, generated
bindings and boundary checks are all exercised on a realistic project. It is not
compiled as Rust here because it has no `Cargo.toml` of its own — the equivalent
code is compiled and executed by
`next-rs/crates/next-rs/tests/complete_example.rs`.
