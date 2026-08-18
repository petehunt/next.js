# `blog-rs` — `examples/blog-starter`, ported to Rust

[`examples/blog-starter`](../../../examples/blog-starter) is the canonical
Next.js App Router example: a Markdown blog that reads `_posts/*.md`, parses
front matter with `gray-matter`, converts Markdown with `remark`, and renders
seventeen React components.

This is that application with the server side moved to Rust. It is a working
port, not a sketch: `cargo run -p blog-rs` serves it, 78 tests cover it, and
`benchmarks/` measures it against the original.

## What moved, and what did not

| `blog-starter` | `blog-rs` |
|---|---|
| `src/lib/api.ts` — `fs` + `gray-matter` | [`src/content.rs`](src/content.rs) — `tokio::fs` + [`src/frontmatter.rs`](src/frontmatter.rs), cached |
| `src/lib/markdownToHtml.ts` — `remark` + `remark-html` | [`src/markdown.rs`](src/markdown.rs) — `pulldown-cmark` |
| `date-formatter.tsx` — `date-fns` in the browser | [`src/dates.rs`](src/dates.rs) — formatted on the server |
| `src/app/page.tsx` | [`app/route.rs`](app/route.rs) |
| `src/app/posts/[slug]/page.tsx` | [`app/posts/[slug]/route.rs`](app/posts/%5Bslug%5D/route.rs) |
| Tailwind through the Next asset pipeline | [`app/styles.css/route.rs`](app/styles.css/route.rs) |
| 16 presentational components | [`src/views.rs`](src/views.rs) |
| `theme-switcher.tsx` (`"use client"`) | **still React** — [`components/ThemeSwitcher.tsx`](components/ThemeSwitcher.tsx), mounted from Rust |

The last row is the interesting one. Sixteen of the seventeen components were
pure functions of their props: no state, no effects, no event handlers. Those
are exactly the components React is *not* needed for, and they became Rust
string templates.

`ThemeSwitcher` is not like that. It holds state, reads and writes
`localStorage`, and subscribes to `matchMedia` and cross-tab `storage` events.
None of that can be produced ahead of time on a server, so it stays a React
Client Component — declared in `next-rs.components.ts`, given its props by a
Rust loader, and mounted in the browser with no server-side JavaScript
whatsoever (§37, §38, §80).

[`components/SubscribeForm.tsx`](components/SubscribeForm.tsx) is new. The
original has exactly one Client Component, so a straight port would only ever
exercise one of the four call-site policies; this one covers `.swr(...)`.

It is deliberately *not* on the served pages. Adding it would change the document
and break parity with the original, and an `.ssr()` call site would need the React
renderer process — which this deployment does not run, and which the benchmark's
"no Node in the request path" claim depends on.
[`tests/serves_the_blog.rs`](tests/serves_the_blog.rs) drives the real refresh
endpoint against it instead.

## Running it

```bash
cd next-rs
cargo run --release -p blog-rs          # http://127.0.0.1:3001
NEXT_RS_PORT=8080 cargo run --release -p blog-rs
```

There is no Node in the request path. The whole deployment is one binary.

## Testing it

```bash
cargo test -p blog-rs -p blog-rs-exports
```

78 tests: the front-matter subset parser, the date formatter, the Markdown
pipeline, the content cache, every view, the loaders, the `#[export]` functions,
and [`tests/serves_the_blog.rs`](tests/serves_the_blog.rs), which drives the real
`NextRsApp` pipeline — routing, slot transform, frame emission, the refresh
endpoint, the 404 path, and §80 with a renderer that panics if it is ever
called.

## Layout

```text
blog-rs/
├── app/
│   ├── route.rs                  GET /
│   ├── posts/[slug]/route.rs     GET /posts/:slug
│   └── styles.css/route.rs       GET /styles.css
├── components/
│   ├── ThemeSwitcher.tsx         ported from blog-starter, still React
│   └── SubscribeForm.tsx         new, for .swr() coverage
├── exports/                      #[export] functions, in a wasm32-buildable crate
├── _posts/                       the same three Markdown files
├── next-rs.components.ts         the component registry (§24)
└── src/
    ├── content.rs                Rust data fetching, cached
    ├── frontmatter.rs            the YAML subset the posts use
    ├── markdown.rs               CommonMark → HTML
    ├── dates.rs                  the `date-fns` format, server-side
    ├── views.rs                  the documents
    ├── react.rs                  the #[react_component] loaders
    └── bin/serve.rs              the native server
```

The route handlers live under `app/`, in the Next filesystem layout, and are
pulled into the crate from there with `#[path]`. Keeping them where the
framework expects them is the point; `#[path]` is what lets them also be
ordinary Rust modules.

## Deliberate differences from the original

These are the places where the port does not produce byte-identical output.
Each is a consequence of a real difference between the two runtimes, and each is
covered by a test.

* **No `next/image`.** `<Image>` needs the Next server to resize on demand; a
  Rust-owned route has no such endpoint. The port emits a plain `<img>` carrying
  the same `width`/`height` the original passes to `<Image>`, which is the part
  that reserves layout space.
* **Hand-written CSS instead of Tailwind.** The class names in the markup are
  unchanged, so the two documents can be diffed. Running Tailwind over a
  Rust-owned document works fine — the utilities are just text in `class`
  attributes — but it would add a Node build step to an example whose point is
  that it does not need one.
* **The no-FOUC script is in `<head>`.** In the original it is rendered by
  `ThemeSwitcher` through `dangerouslySetInnerHTML`. Rust owns the document, so
  it can put the script where it actually belongs, which is earlier than any
  slot can mount.
* **`&lt;` where `remark` writes `&#x3C;`.** The same character to a browser.
  Parity is checked on rendered text, not on bytes.
* **A front-matter subset, not a YAML engine.** Every post uses the same six
  keys with the same two shapes, so `src/frontmatter.rs` reads exactly that and
  fails loudly on anything else. An unparseable post should stop the build, not
  render as a blank page.

Raw HTML in a post is **dropped**, not passed through — which is what
`remark-html` does by default, so this one is parity rather than a difference.

## Benchmarks

[`next-rs/benchmarks`](../../benchmarks) measures this port against the original
on requests per second, resident memory and cold build time, with ten isolated
runs per variant. The methodology, the individual runs and the environment
caveats are all recorded there.
