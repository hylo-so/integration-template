# Integration Status

Branch `feat/hylo-v2`, 2026-07-06. Titan venue integration for Hylo V2,
routed through `hylo-router` (1:1 with `resolve_route`), quoting through
`hylo-quotes` `ProtocolState`. SDK pinned to git branch
`debt/heap-allocations` (`e20b577`).

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

RPC-gated (`make test-venue` against mainnet shadow), latest observed
results — runs only complete when the shadow price pushers are inside the
oracle windows (see problem 5):

| Test | Status | Cause |
| --- | --- | --- |
| `random_samples` | pass | off-chain quote == on-chain output exactly |
| `monotone` | pass | |
| `zero_input_spot_price` | pass | |
| `construction` (no-alloc) | fail | one 64-byte alloc; see problem 2 |
| `bound_simulation` | fail | oversized quotes execute-fail; see problem 1 |
| `quoting_speed` | fail | 1.09–1.57µs vs the 1µs budget; see problem 3 |
| `price_monotone` | fail | ~0.1% local convexity; see problem 4 |
| `mean_value_theorem` | fail | same as above |
| `hylo_route` (program-side) | fail | same root cause as `bound_simulation` |

The passing trio is the strongest signal: quoting is byte-exact against the
deployed programs across random sizes and all directions sampled. The exo /
USDC / earn-pool directions haven't had a full sim pass yet — blocked
behind problems 1 and 5.

## Outstanding problems

1. **Size bounds vs pure SDK quotes** (fails `bound_simulation`,
   `hylo_route`). The SDK ops don't model execution-side liquidity caps:
   LST vault balances (LST↔LST, redeems), the virtual stablecoin burn floor
   (`supply − 0.1 hyUSD`), and equivalents for the USDC/exo vaults. Titan's
   bounds engine derives routable ranges from `quote()`, so quotes beyond
   those caps fail at execution (`insufficient funds`, `BurnUnderflow`,
   `VirtualStablecoinBurnLimit` — all observed). Decision standing: no
   venue-side cap hacks. Resolution is either modeling the caps in the SDK
   `TokenOperation`s (quotes become execution-accurate for every consumer)
   or an understanding with Titan. Titan's spec text does expect quotes to
   reflect liquidity.

2. **No-alloc quote path** (fails `construction`). The
   `debt/heap-allocations` sweep cleaned hylo-core (was a 13-byte
   `String`, anchor error names). One allocation remains:
   `hylo-quotes::TokenOperation::compute_output` returns `anyhow::Result`,
   so hylo-core's `CoreError` gets heap-boxed at that boundary — Titan's
   bounds search probes oversized amounts, hits error paths, guard aborts
   (64 bytes). Fix is in hylo-quotes: error type `CoreError` instead of
   `anyhow::Error`; venue then drops `anyhow` from `src/hylo/quote.rs`.
   Note problem 1's resolution also determines how often error paths fire
   at all.

3. **Quote speed** (fails `quoting_speed`). Budget 1µs average; measured
   1.09–1.57µs depending on direction. Each quote runs two evaluations
   (output + marginal price secant) and the SDK recomputes NAVs/conversions
   per call. Levers, in rough order of value: the problem-2 fix (error
   boxing costs on probe-heavy tests), amount-independent precompute inside
   hylo-quotes (NAVs, conversions, mode — all state-constant), thin LTO in
   this repo (two lines, removed during cleanup), or Titan tolerance.

4. **Local convexity** (fails `price_monotone`, `mean_value_theorem`).
   Hylo's collateral-ratio fee curves have non-monotone segment slopes and
   mode-tier steps, so output is locally convex (~0.1%) — over Titan's
   1e-3 monotonicity and 1e-5 mean-value tolerances, loud on low-TVL
   shadow. Not honestly fixable in quote code while quotes must equal
   execution. Decision standing: settle tolerance/handling directly with
   the Titan team. Numbers for that conversation: monotone violation
   +0.105% at ~$11 trade size; MVT overshoot ~5e-5.

5. **Shadow oracle freshness** (intermittently blocks every RPC-gated
   test). Both SOL/USD (`7AviUf9n...`) and BTC/USD (`APgz...`) get pushed
   every ~25–30s against 10s `oracle_interval_secs` windows; state load
   fails `PythOracleOutdated` whenever the snapshot lands stale.
   `ProtocolState` builds the cbBTC exo context unconditionally, so the
   BTC feed blocks even LST-only quoting. Fixes: push cadence under 10s,
   widen the shadow intervals (max 60s), and/or an LST-scoped quote state
   in hylo-quotes (`fetch_lst_context` exists; the `TokenOperation` impls
   hang off `ProtocolState` only). The posted-slot validation fix is on
   the pinned branch but the deployed shadow programs predate it.

6. **SDK branch state.** `debt/heap-allocations` is unmerged; this repo
   and the program template both pin it. Re-pin to `main-v2` (or a tag)
   once merged. `hylo-fix` 0.7.0 came with it.

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
