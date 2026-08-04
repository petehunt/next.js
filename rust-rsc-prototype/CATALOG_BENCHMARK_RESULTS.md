# Catalog benchmark results

Generated 2026-08-04T08:40:00Z after the structured router-cache/navigation and task-graph work. Local directional results from four alternating ABBA/BAAB blocks; see `catalog/benchmark-raw.json` for latency, TTFB, CPU, RSS, byte, upstream, cancellation, and all per-run data.

Correctness gates passed immediately before measurement: four HTML/decoded-Flight parity cases, identical ordered product IDs and stylesheet hash, identical upstream request counts, production browser hydration with zero errors, independently timed loading regions, and deferred navigation Flight decoded by the vendored React client. Both arms used the same production Next build, deterministic local service, seed, query parameters, cache modes, request counts, and alternating schedule. Confidence intervals below use eight runs per arm and small-sample Student-t critical values. They remain machine-local evidence, not fleet-level claims.

| Scenario                       | JavaScript RPS (95% CI) | Rust RPS (95% CI) | Rust / JS |
| ------------------------------ | ----------------------: | ----------------: | --------: |
| document-c1-zero-uncached      |            111.0 ± 21.2 |        76.3 ± 8.2 |     0.69× |
| document-c8-zero-uncached      |            149.7 ± 10.3 |      497.2 ± 81.4 |     3.32× |
| document-c32-zero-uncached     |             165.8 ± 9.6 |    1329.3 ± 127.3 |     8.02× |
| document-c8-latency25-uncached |             147.8 ± 6.3 |       197.2 ± 9.0 |     1.33× |
| document-c8-zero-warm          |            211.2 ± 13.7 |      510.7 ± 49.4 |     2.42× |
| flight-c8-zero-uncached        |            239.3 ± 24.1 |      517.3 ± 68.2 |     2.16× |

## Latency, resources, and bytes

Values are the mean of eight runs for each arm. CPU is process CPU consumed during one bounded run; RSS is the post-run resident set.

| Scenario                    |       JS p50 / p95 / p99 |  Rust p50 / p95 / p99 | JS / Rust TTFB p50 | JS / Rust CPU |        JS / Rust RSS | JS / Rust bytes |
| --------------------------- | -----------------------: | --------------------: | -----------------: | ------------: | -------------------: | --------------: |
| document c1 uncached        |     7.7 / 16.1 / 29.8 ms | 12.6 / 18.6 / 19.3 ms |       7.5 / 3.8 ms |   273 / 34 ms |  172,354 / 5,568 KiB | 30,143 / 25,258 |
| document c8 uncached        |    50.5 / 65.2 / 68.2 ms | 14.0 / 23.2 / 24.2 ms |      49.9 / 4.7 ms |   246 / 35 ms |  202,319 / 8,756 KiB | 30,138 / 25,258 |
| document c32 uncached       | 188.0 / 208.0 / 213.4 ms | 19.9 / 38.5 / 43.8 ms |     188.0 / 6.9 ms |  873 / 176 ms | 328,135 / 17,937 KiB | 30,138 / 25,258 |
| document c8, 25 ms upstream |    48.6 / 67.6 / 71.7 ms | 38.4 / 44.5 / 46.8 ms |      27.7 / 4.4 ms |   240 / 39 ms | 415,352 / 18,170 KiB | 30,213 / 25,292 |
| document c8 warm cache      |    35.9 / 48.0 / 51.8 ms | 14.1 / 20.1 / 21.1 ms |      35.9 / 4.6 ms |   166 / 25 ms | 476,344 / 18,257 KiB | 28,772 / 25,250 |
| Flight c8 uncached          |    30.3 / 45.5 / 48.0 ms | 14.0 / 20.2 / 21.2 ms |      20.3 / 4.5 ms |   149 / 36 ms | 516,504 / 18,114 KiB | 14,726 / 12,794 |

Every uncached bounded run made exactly two upstream requests per document/navigation. Warm runs made zero upstream requests after the explicit prime on both arms. Aborting a 1-second request at about 60 ms left both processes healthy; the next JavaScript request completed in 21.5 ms and Rust in 21.8 ms.

## Cold process and cold data cache

`catalog/benchmark-cold-raw.json` contains eight fresh-process runs per arm in four alternating ABBA/BAAB blocks, with a content gate on every response and small-sample Student-t 95% intervals. JavaScript reached readiness in 226.3 ± 11.5 ms and completed its first cold document in 610.1 ± 31.3 ms from process spawn. Rust reached readiness in 2.03 ± 0.26 ms and completed its first cold document in 22.2 ± 4.0 ms from process spawn. The modal response sizes were 30,169 and 25,250 bytes respectively.

Exact commands:

```bash
BENCH_BLOCKS=4 CATALOG_JS_PID=<next-server-pid> CATALOG_RUST_PID=<native-pid> node rust-rsc-prototype/catalog/benchmark.js
BENCH_BOOT_BLOCKS=4 node rust-rsc-prototype/catalog/benchmark-cold.js
```
