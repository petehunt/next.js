# Catalog architecture benchmark

Generated: 2026-08-05T01:13:05.327Z

Platform: Intel(R) Core(TM) i7-10700 CPU @ 2.90GHz, 16 logical CPUs, x64, Node v22.18.0.

Method: 5 alternating local phases × 2000 ms at concurrency 1 and 32. Browser navigation uses 12 warm-cache samples.

Important limitation: Single-host exploratory benchmark; repeated phases are not independent VM boots and are not confidence intervals.

## Readout

- At concurrency 32, Node-free native throughput is 9.06× conventional Next/JS for the list document, 14.49× for detail, and 5.59× for Flight.
- At concurrency 1, Node-free native list throughput is 0.45× conventional Next/JS; this includes connection acceptance, rendering, and response streaming.
- Native list TTFB at concurrency 1 is 1.2 ms versus 3.85 ms for Next/JS, while full streamed-body completion is slower (8.05 ms versus 3.93 ms).
- The hybrid detail route reaches 1.16× conventional throughput at concurrency 32, while hybrid list Flight reaches 0.71×.
- The selective-fallback topology ends at 425.13 MiB across Rust + Node, while Node-free native ends at 19.11 MiB.
- Proxying a JS route through the Rust fallback front at concurrency 32 retains 78.95% of direct throughput.
- These are exploratory magnitudes from repeated phases on one host, not boot-level statistical claims.

## list-document

| configuration                            | concurrency |   req/s |       p50 |       p95 |  TTFB p50 | bytes | RPS CV |
| ---------------------------------------- | ----------: | ------: | --------: | --------: | --------: | ----: | -----: |
| Conventional Next/JS                     |           1 |  229.64 |   3.93 ms |   6.15 ms |   3.85 ms | 31055 |  8.16% |
| Next + Rust/Wasm hybrid                  |           1 |  191.37 |   4.84 ms |   6.98 ms |   4.76 ms | 30357 |  6.57% |
| Native Rust with selective Next fallback |           1 |  103.06 |   8.05 ms |   12.1 ms |   1.16 ms | 27931 |  2.22% |
| Node-free native Rust                    |           1 |  102.87 |   8.05 ms |  12.08 ms |    1.2 ms | 27931 |  1.25% |
| Conventional Next/JS                     |          32 |  290.54 | 106.85 ms | 132.65 ms |  106.8 ms | 31055 |  2.17% |
| Next + Rust/Wasm hybrid                  |          32 |  237.73 | 130.99 ms | 157.29 ms | 130.91 ms | 30357 |  4.27% |
| Native Rust with selective Next fallback |          32 | 2626.22 |  11.97 ms |  17.93 ms |    3.2 ms | 27931 |  3.42% |
| Node-free native Rust                    |          32 | 2632.57 |  11.88 ms |  18.25 ms |   3.18 ms | 27931 |  3.27% |

## filtered-document

| configuration                            | concurrency |   req/s |       p50 |       p95 |  TTFB p50 | bytes | RPS CV |
| ---------------------------------------- | ----------: | ------: | --------: | --------: | --------: | ----: | -----: |
| Conventional Next/JS                     |           1 |  262.95 |   3.45 ms |   5.49 ms |   3.38 ms | 31103 |  3.47% |
| Next + Rust/Wasm hybrid                  |           1 |   227.5 |    4.1 ms |   5.97 ms |   4.02 ms | 30418 |  2.13% |
| Native Rust with selective Next fallback |           1 |   102.7 |   8.04 ms |  12.07 ms |   1.05 ms | 27949 |  3.41% |
| Node-free native Rust                    |           1 |  103.07 |   8.05 ms |  12.08 ms |   1.18 ms | 27949 |   2.5% |
| Conventional Next/JS                     |          32 |  316.73 |  98.35 ms | 120.15 ms |  98.29 ms | 31103 |  1.55% |
| Next + Rust/Wasm hybrid                  |          32 |  257.86 | 119.35 ms | 151.15 ms | 119.29 ms | 30418 |  5.16% |
| Native Rust with selective Next fallback |          32 | 2539.65 |  12.27 ms |  18.88 ms |   3.58 ms | 27949 |  2.33% |
| Node-free native Rust                    |          32 | 2554.01 |  12.18 ms |  18.96 ms |   3.61 ms | 27949 |  4.55% |

## detail-document

| configuration                            | concurrency |   req/s |      p50 |       p95 | TTFB p50 | bytes | RPS CV |
| ---------------------------------------- | ----------: | ------: | -------: | --------: | -------: | ----: | -----: |
| Conventional Next/JS                     |           1 |  269.02 |  3.47 ms |   5.19 ms |  3.41 ms |  9288 |  2.91% |
| Next + Rust/Wasm hybrid                  |           1 |  322.37 |   2.8 ms |   4.56 ms |  2.73 ms |  9447 |  4.68% |
| Native Rust with selective Next fallback |           1 |  859.56 |  1.12 ms |   1.26 ms |  1.09 ms |  4171 |  1.14% |
| Node-free native Rust                    |           1 |  866.53 |  1.11 ms |   1.26 ms |  1.08 ms |  4171 |  1.76% |
| Conventional Next/JS                     |          32 |  361.61 | 85.21 ms | 108.49 ms | 85.17 ms |  9288 |  4.68% |
| Next + Rust/Wasm hybrid                  |          32 |  420.51 | 74.58 ms |   94.4 ms | 74.53 ms |  9447 |  1.65% |
| Native Rust with selective Next fallback |          32 | 5136.36 |  5.79 ms |   9.42 ms |  5.77 ms |  4171 |   2.3% |
| Node-free native Rust                    |          32 | 5241.45 |  5.75 ms |   8.97 ms |  5.73 ms |  4171 |   1.5% |

## list-flight

| configuration                            | concurrency |   req/s |      p50 |      p95 | TTFB p50 | bytes | RPS CV |
| ---------------------------------------- | ----------: | ------: | -------: | -------: | -------: | ----: | -----: |
| Conventional Next/JS                     |           1 |  423.74 |  1.96 ms |  4.59 ms |  1.89 ms | 15981 |  4.27% |
| Next + Rust/Wasm hybrid                  |           1 |  329.04 |  2.77 ms |  4.85 ms |  2.69 ms | 15547 |  3.61% |
| Native Rust with selective Next fallback |           1 |  103.49 |  8.07 ms |  12.1 ms |  1.19 ms | 13984 |  1.74% |
| Node-free native Rust                    |           1 |  101.87 |  8.07 ms | 12.07 ms |   1.1 ms | 13984 |  2.07% |
| Conventional Next/JS                     |          32 |  565.91 | 54.59 ms | 71.45 ms | 54.52 ms | 15981 |  3.88% |
| Next + Rust/Wasm hybrid                  |          32 |  399.52 |  78.2 ms | 99.68 ms | 78.13 ms | 15547 |   0.9% |
| Native Rust with selective Next fallback |          32 | 3115.22 | 10.45 ms | 15.83 ms |  2.47 ms | 13984 |  1.07% |
| Node-free native Rust                    |          32 | 3163.57 | 10.28 ms | 15.49 ms |  2.38 ms | 13984 |  0.96% |

## Browser list → detail navigation

| configuration                            |   median |      p95 |
| ---------------------------------------- | -------: | -------: |
| Conventional Next/JS                     | 65.96 ms | 77.54 ms |
| Next + Rust/Wasm hybrid                  | 62.57 ms | 68.03 ms |
| Native Rust with selective Next fallback | 61.03 ms | 66.51 ms |
| Node-free native Rust                    | 61.56 ms | 63.22 ms |

## Resident memory

| configuration                            |     before |      after | process count |
| ---------------------------------------- | ---------: | ---------: | ------------: |
| Conventional Next/JS                     | 177.77 MiB | 761.62 MiB |             1 |
| Next + Rust/Wasm hybrid                  | 169.25 MiB | 236.88 MiB |             1 |
| Native Rust with selective Next fallback | 171.52 MiB | 425.13 MiB |             2 |
| Node-free native Rust                    |    5.5 MiB |  19.11 MiB |             1 |

## Selective fallback overhead

| concurrency | direct req/s | proxied req/s | direct p50 | proxied p50 |
| ----------: | -----------: | ------------: | ---------: | ----------: |
|           1 |       258.55 |        170.68 |    3.52 ms |     5.45 ms |
|          32 |       310.27 |        244.95 |  100.31 ms |    129.2 ms |
