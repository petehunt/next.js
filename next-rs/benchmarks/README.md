# `blog-rs` versus `blog-starter`

A like-for-like comparison of [`next-rs/examples/blog-rs`](../examples/blog-rs)
against the application it was ported from,
[`examples/blog-starter`](../../examples/blog-starter): the same three Markdown
posts, the same URLs, the same rendered pages.

Three things are measured — requests per second, resident memory, cold build
time — with ten isolated runs per variant, plus a functional parity check,
because a throughput number for a server that renders the wrong page is
worthless.

[`results/RESULTS.md`](results/RESULTS.md) holds the numbers from the run
recorded in this branch, and [`results/results.json`](results/results.json) holds
every individual sample behind them.

Headline, from ten runs each on 4 vCPUs:

| | `blog-rs` | `blog-starter` | |
|---|---:|---:|---|
| `/` throughput | 17,984 req/s | 2,581 req/s | **7.0×** |
| `/posts/[slug]` throughput | 17,773 req/s | 2,267 req/s | **7.8×** |
| `/` latency p50 | 0.70ms | 5.59ms | **8.0×** |
| peak RSS under load | 5.5 MiB | 213.2 MiB | **39×** |
| rebuild after an app change | 2.51s | 8.01s | **3.2×** |
| build with no dependency cache | 20.9s | 8.1s | 0.4× |

Relative standard deviation across runs was 0.6–2.4%, so those medians are
separated by far more than the noise. Read the caveats at the bottom before
quoting any of it — in particular, `blog-starter` is serving *prerendered* HTML
here, which is close to its best case.

## Reproducing it

```bash
# 1. Build the Rust port.
cd next-rs && cargo build --release -p blog-rs && cd ..

# 2. Prepare the baseline. Copies the example out of the monorepo and installs
#    published Next.js — see below for why that matters.
node next-rs/benchmarks/setup.mjs --to /tmp/bench/blog-starter

# 3. Build the baseline once, so `next start` has something to serve.
(cd /tmp/bench/blog-starter && NODE_ENV=production ./node_modules/.bin/next build)

# 4. Run.
node next-rs/benchmarks/run.mjs --runs 10 --next-app /tmp/bench/blog-starter
```

Useful flags: `--runs N`, `--duration MS`, `--concurrency N`, `--out DIR`, and
`--skip-parity` / `--skip-throughput` / `--skip-compile` for iterating on one
measurement at a time.

## Methodology

### Parity first

[`lib/parity.mjs`](lib/parity.mjs) fetches every page from both servers and
compares the **visible text**, the headings, the internal links, the rendered
dates and the status code.

It deliberately does *not* compare bytes. One document is built by React and
Tailwind and the other by Rust string templates, so they differ in whitespace,
attribute order, `&#x3C;` versus `&lt;`, React's hydration comment markers, and
the framework's own script and preload tags. None of that is what "the same blog"
means, and normalising it away silently would be worse than saying so.

Differences that *are* real are listed in `EXPECTED_DIFFERENCES` with the reason,
so they appear in the report rather than being hidden. There is currently one, and
it is worth reading: `blog-starter` answers an unknown slug with a **500**,
because `getPostBySlug` calls `fs.readFileSync` and lets `ENOENT` propagate, which
makes its own `if (!post) notFound()` unreachable. The port returns `None` and
renders a 404 page — what the original was trying to do.

The parity check found a real porting bug, too: `alert.tsx` renders on *every*
post page, not only previews, and the port had omitted its non-preview branch.

### Throughput

[`lib/load.mjs`](lib/load.mjs) drives 16 keep-alive connections at one URL for
five seconds, after a 1.5-second warmup, reading every response body to
completion.

* **The warmup is not cosmetic.** Next's first requests populate its caches, and
  the Rust server's first request fills its Markdown parse cache. Measuring those
  would be measuring start-up.
* **Bodies are read fully.** A server that streams would otherwise look faster
  than it is, because only the first byte would be timed.
* **The generator is hand-written.** A benchmark whose harness needs
  `npm install` is a benchmark nobody reproduces, and — more importantly — the
  load generator shares the four vCPUs with both servers, so its cost has to be
  cheap and *identical* for both variants.

Each run is a fresh server process. The variants alternate, so any drift over the
session lands on both.

### Memory

Resident memory is sampled every 100ms during load and summed over the whole
process tree. `next start` is a supervisor that forks a render worker, so asking
the parent alone reports a few tens of megabytes while the work happens elsewhere;
the tree total is the only figure comparable to a single Rust binary.

That sum double-counts shared pages, which overstates the Node tree somewhat.
Correcting it properly needs `smaps_rollup` per process, and the correction is far
smaller than the gap being measured — so it is called out rather than applied.

### Cold build

`cargo build --release -p blog-rs` after `cargo clean -p blog-rs`, versus
`next build` after `rm -rf .next`.

Both rebuild the *application* and reuse their dependency caches — Cargo's
`target/` for one, `node_modules` for the other. Neither includes downloading
dependencies. This is the comparison a developer actually experiences after
changing application code, and it is the one a warm CI cache reproduces.

**The from-scratch numbers are less flattering, and belong here too.** After
`cargo clean --release`, rebuilding every dependency takes **20.9s** on this
machine — 2.6× `next build` rather than 0.31×. `next build` after
`rm -rf .next node_modules/.cache` takes **8.1s**, essentially unchanged, because
Next's dependencies arrive precompiled from npm while Cargo's arrive as source.
So: Rust wins the edit-rebuild loop by roughly 3×, and loses a cold CI build with
no cache by roughly 2.6×.

## Why runs are sequential

`--parallel` exists and is off by default, and it should stay off. Two servers
competing for the same four vCPUs do not produce two independent measurements;
they produce two wrong ones. The same applies to the compile measurements, where
`cargo` and `next build` are both happy to use every core.

What *is* parallelised safely is setup: `setup.mjs` and the Rust build are
independent and can run at the same time.

## Environment caveats

Read these before quoting any number.

* **One machine shape.** These runs are from a Vercel sandbox — a Firecracker
  microVM with 4 vCPUs and 8 GiB — with the load generator on the same host. The
  absolute numbers describe that shape and nothing else. The **ratios** are the
  transferable part, and even those will move with core count: a Node server
  scales across cores through workers, and giving it more of them narrows the
  throughput gap.
* **Loopback, not a network.** No TLS, no real latency, no proxy. Both servers
  benefit equally, but it does mean per-request cost dominates in a way it would
  not behind a CDN.
* **`blog-starter` is statically generated.** Next is serving prerendered HTML
  from disk for `/` and the three posts — close to its best case. The Rust port
  renders every response, including running the Markdown pipeline for a post page,
  and still serves the parse from cache. That asymmetry favours Next and is worth
  keeping in mind: this is not "Rust templating versus React rendering", it is
  "a Rust binary versus Node serving prerendered files".
* **The port ships fewer features.** No `next/image` optimizer, no Tailwind build,
  no route prefetching, no App Router client-side navigation. Some of that is
  irrelevant to what is being measured; some of it is genuine functionality the
  Rust port does not have. [`examples/blog-rs/README.md`](../examples/blog-rs/README.md)
  lists it.
* **A Node process is not idle at 5 MiB.** Part of the memory difference is the
  V8 heap and the React runtime existing at all, which is the point — but a
  server tuned with `--max-old-space-size` and no render worker would land
  somewhere between the two figures.
* **Single sandbox, not cross-host.** Isolation here is process-level within one
  microVM: fresh process per run, alternating variants, ten repetitions, and a
  reported relative standard deviation so a reader can judge whether a difference
  in medians means anything. It is not isolation between physical hosts, and no
  claim is made about run-to-run variance across different machines.
