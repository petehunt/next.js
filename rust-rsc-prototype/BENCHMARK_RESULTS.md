# Rust RSC prototype benchmark

Measured August 4, 2026 on the local development machine using the production Webpack build, an optimized Rust runtime, 32 concurrent keep-alive clients, 2.5-second phases, and an ABBA arm order. Both arms served the same `/rust-page` logical route. Flight requests used the canonical `_rsc` URL and router-state header selected by Next.

This is a directional prototype benchmark, not a Vercel Sandbox confidence result. The arms are different runtime architectures and payload shapes, and one machine boot is not a statistical unit of replication. Payload size is part of the observed effect: the native restricted payload intentionally omits unsupported Next semantics.

## First run

| response | runtime | requests/sec |     mean |      p50 |      p95 | bytes |
| -------- | ------: | -----------: | -------: | -------: | -------: | ----: |
| document |    Next |     1,818.69 | 17.52 ms | 15.21 ms | 30.35 ms | 4,994 |
| document |    Rust |     5,380.14 |  5.94 ms |  5.67 ms |  7.85 ms | 1,801 |
| Flight   |    Next |     1,646.72 | 19.36 ms | 18.38 ms | 24.90 ms | 3,411 |
| Flight   |    Rust |     5,877.72 |  5.44 ms |  5.30 ms |  6.62 ms |   778 |

## Independent confirmation

| response | runtime | requests/sec |     mean |      p50 |      p95 | bytes |
| -------- | ------: | -----------: | -------: | -------: | -------: | ----: |
| document |    Next |     2,333.46 | 13.65 ms | 12.79 ms | 19.81 ms | 4,994 |
| document |    Rust |     5,195.67 |  6.15 ms |  5.78 ms |  8.34 ms | 1,801 |
| Flight   |    Next |     1,837.30 | 17.35 ms | 17.02 ms | 21.06 ms | 3,411 |
| Flight   |    Rust |     5,633.38 |  5.67 ms |  5.47 ms |  6.98 ms |   778 |

All measured responses returned HTTP 200. Across both runs, the native arm delivered 2.23–2.96× document throughput and 3.07–3.57× Flight throughput in this local setup. Treat those ranges as motivation for a remote boot-level experiment after the prototype becomes a comparable Next.js revision, not as production forecasts.

Re-run with:

```bash
node rust-rsc-prototype/benchmark-local.js
```
