# Integration Status

Branch `feat/hylo-v2`, 2026-07-07. Titan venue integration for Hylo V2,
routed through `hylo-router` (1:1 with `resolve_route`), quoting through
`hylo-quotes` `ProtocolState`. SDK pinned to git branch
`debt/heap-allocations` (`6214ac3`).

## Shape

- 8 tokens: jitoSOL, hyloSOL, hyUSD, xSOL, sHYUSD, USDC, cbBTC, xBTC.
- 28 directions from `ROUTABLE_PAIRS` (`src/hylo/mod.rs`), mirroring the
  router's pair table exactly.
- Every swap leg is one `hylo-router` `route` instruction; account contexts
  from `hylo-idl` builders per pair; on-chain adapter
  (`program-template/.../venues/hylo_router.rs`) delegates to the SDK
  instruction builder. Program ids, discriminators, and layouts all come
  from the IDL.
- Deployment: `shadow` cargo feature (default on) targets the live mainnet
  V2 shadow programs; the canonical ids still serve V1.

## Test matrix

No-RPC (all green, run per commit):

| Check | Status |
| --- | --- |
| `make check-structure` (lib tests, scorecard, enum parity) | ok |
| Scorecard layers (creation / quote / program / route builder) | 4/4 |
| `tests/hylo_creation.rs` (register_lst fixtures) | ok |
| `build-program` (SBF via `#sbf` shell) | ok |
| `polish` / `lint` | ok |

RPC-gated (`make test-venue` against mainnet shadow), observed 2026-07-07.
**Every RPC test currently fails, and all failures trace to shadow protocol
state, not venue code** (see problem 0):

| Test | Status | Cause |
| --- | --- | --- |
| `construction` | fail | unquotable directions; problem 0 |
| `zero_input_spot_price` | fail | dead directions quote price 0; problem 0 |
| `monotone` | fail | bounds search finds no quotable value; problem 0 |
| `quoting_speed` | fail | dies in bounds before timing; problem 0 |
| `random_samples` | fail | `SwapLstToLst` insufficient vault funds; problem 1 |
| `bound_simulation` | fail | same; problems 0 and 1 |
| `price_monotone` | fail | ~0.1% local convexity; problem 4 |
| `mean_value_theorem` | fail | same as above |
| `hylo_route` (program-side) | fail | same root causes |

Progress since 2026-07-06: the no-alloc quote path is **done** (old problem
2). SDK `6214ac3` returns `CoreError` by value from all token operations,
and the venue dropped `anyhow` from `src/hylo/quote.rs` (`66474cc`). Under
the alloc guard the quote path is clean; the previously-passing trio
(`random_samples`, `monotone`, `zero_input_spot_price`) regressed for
state reasons only.

## Outstanding problems

0. **Shadow protocol state is wedged** (new; currently fails everything).
   The shadow deployment's collateral ratio sits outside the rebalance
   pricing-curve domain, and the BTC exo pair is in SellZone2/Depeg.
   Measured quote-level effects (SDK gates, matching on-chain behavior):
   * USDC → jitoSOL / hyloSOL / cbBTC: unquotable at **every** size
     ("CR is outside the rebalance pricing curve domain").
   * cbBTC → USDC: dead above ~$0.01; cbBTC → hyUSD: dead above ~$60
     ("No valid mint fee for stablecoin due to SellZone2 or Depeg").
   * hyUSD → LST/xSOL/exo: caps of a few dollars (tiny hyUSD supply).
   The venue advertises all 28 directions, so `zero_input_spot_price`
   reports price 0 and Titan's bounds engine finds no quotable value
   (`NoQuotableValue` → `u64::MAX`), which also kills `construction`,
   `monotone`, and `quoting_speed` before they measure anything.
   Fix is protocol-side: recapitalize/rebalance the shadow deployment
   (or point the suite at state whose CR is inside the curve domain).
   Open venue question for Titan: what should a venue report for a
   direction that is state-disabled — drop it from `directions_num`, or
   quote `not_enough_liquidity` at all sizes (current behavior, which
   their suite rejects)?

1. **Size bounds vs pure SDK quotes** (fails `random_samples`,
   `bound_simulation`, `hylo_route`). The SDK ops don't model
   execution-side liquidity caps: LST vault balances (LST↔LST, redeems),
   the virtual stablecoin burn floor (`supply − 0.1 hyUSD`), and
   equivalents for the USDC/exo vaults. Titan's bounds engine derives
   routable ranges from `quote()`, so quotes beyond those caps fail at
   execution (`insufficient funds` on `SwapLstToLst` observed again
   2026-07-07). Decision standing: no venue-side cap hacks. Resolution is
   either modeling the caps in the SDK `TokenOperation`s or an
   understanding with Titan. Titan's spec text does expect quotes to
   reflect liquidity.

2. ~~No-alloc quote path~~ **Done 2026-07-07** (SDK `6214ac3` + venue
   `66474cc`). One residue, Titan-side: when a direction is unquotable,
   the template's bounds engine `log::error!`s inside the
   `assert_no_alloc` region and env_logger's buffer allocation aborts the
   whole process with no location (their `assert_no_alloc` pin strips the
   `backtrace`/`warn_debug` features). `RUST_LOG=off` sidesteps it; worth
   reporting upstream.

3. **Quote speed** (blocked from measuring). Budget 1µs average; last
   measured 1.09–1.57µs. The error-boxing cost is gone with problem 2;
   remeasure once problem 0 unblocks the bounds engine. Remaining levers:
   amount-independent precompute inside hylo-quotes (NAVs, conversions,
   mode), thin LTO in this repo, or Titan tolerance.

4. **Local convexity** (fails `price_monotone`, `mean_value_theorem`).
   Hylo's collateral-ratio fee curves have non-monotone segment slopes and
   mode-tier steps, so output is locally convex (~0.1%) — over Titan's
   1e-3 monotonicity and 1e-5 mean-value tolerances, loud on low-TVL
   shadow. Not honestly fixable in quote code while quotes must equal
   execution. Decision standing: settle tolerance/handling directly with
   the Titan team. Fresh numbers: +0.11% at ~$12 trade size
   (0.10390 → 0.10401 on jitoSOL→hyUSD).

5. **Shadow oracle freshness** (intermittently blocks every RPC-gated
   test; ~3 of 4 attempts on 2026-07-07). Both SOL/USD (`7AviUf9n...`)
   and BTC/USD (`APgz...`) get pushed every ~25–30s against 10s
   `oracle_interval_secs` windows; state load fails whenever the snapshot
   lands stale. `ProtocolState` builds the cbBTC exo context
   unconditionally, so the BTC feed blocks even LST-only quoting. Fixes:
   push cadence under 10s, widen the shadow intervals (max 60s), and/or an
   LST-scoped quote state in hylo-quotes. The posted-slot validation fix
   is on the pinned branch but the deployed shadow programs predate it.

6. **SDK branch state.** `debt/heap-allocations` is unmerged; this repo
   and the program template both pin it (`6214ac3`). Re-pin to `main-v2`
   (or a tag) once merged. `hylo-fix` 0.7.0 came with it.

## Divergences from Titan's upstream template

- Raydium example deleted (example code, not part of the integration);
  scorecard rewritten Hylo-only. Consequence: the shared suites have no
  always-green reference venue running against them.
- Program crate on anchor 0.32.1 (hylo-idl requirement); one migration
  touch in the template's test helper (`solana_program::hash`).
- Host-side program tests run with `CARGO_PROFILE_RELEASE_LTO=off`
  (Makefile): fat LTO force-merges the program's `entrypoint` with the
  litesvm runtime in test binaries. The on-chain build keeps `lto = "fat"`.
- litesvm 0.7 / solana 2.3 stack; `Cargo.lock` files committed (the pyth
  sdk's open-ended anchor requirement otherwise resolves duplicate majors
  needing rustc 1.89).
- Nix flake + shell tools (`lint`, `polish`, `build`, `build-program`,
  `test-structure`, `test-venue`), hylo rustfmt config.

## Running

```bash
nix develop                       # or direnv
make check-structure              # no-RPC checks
build-program                     # SBF build via the #sbf shell
export SOLANA_RPC_URL=https://... # mainnet
make dump-programs                # one-time: exchange, router, earn pool
make test-venue                   # full suite; scorecard at the end
```
