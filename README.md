# Titan Integration — Hylo V2

Hylo's V2 integration for Titan's routing layer, built on Titan's AMM
integration template.

## Hylo V2 venue

Hylo is an LST-collateralized exchange, not a pool AMM: it mints/redeems the
hyUSD stablecoin and the xSOL levercoin against LST collateral, converts
between them, and swaps LST<->LST through its vaults. One venue
(`src/hylo/`, `HyloVenue`) covers all 12 directions over
`[jitoSOL, hyloSOL, hyUSD, xSOL]`; the market account is Hylo's global state
(`pda::HYLO`).

Both halves go through Hylo's standard interfaces, exactly like the Jupiter
integration: quotes load `ProtocolState` from `hylo-quotes` (the same
`hylo-core` math the on-chain program executes), and every swap leg is one
`hylo-router` `route` instruction — the router resolves the exchange
instruction from the mint pair on-chain.

Layers:

- Quote: `src/hylo/` (`HyloVenue`, `HyloOp`, `parse_pool_creations` —
  a "pool creation" is `register_lst`)
- Route builder: `Venue::Hylo { token_a, token_b }` in
  `src/swap_route/mod.rs`
- Program: `program-template/.../instructions/venues/hylo_router.rs`
- Tests: `tests/hylo.rs`, `tests/hylo_creation.rs`,
  `program-template/.../tests/hylo_route.rs`

### Deployment: the `shadow` feature

Hylo V2 currently runs as the mainnet **shadow** deployment
(`hyshEX5sNEYhnYPMm8MwMThhBRPuLN3rjoYDbC9esPQ`); the canonical id
(`HYEXCHtHkBagdStcJCp3xbbb9B7sdMdWXFNj6mdsG4hn`) still serves V1. The
`shadow` cargo feature — **on by default** in both crates — points every
program id and PDA at the live V2 deployment. When V2 is promoted to the
canonical id, drop `shadow` from the `default` feature lists in `Cargo.toml`
and `program-template/programs/titan-v3-venue-template/Cargo.toml` and update
`HYLO_EXCHANGE` / `HYLO_ROUTER` in the `Makefile`.

### Known caveats (shadow, as of 2026-07)

The RPC-gated simulation tests require the shadow SOL/USD Pyth feed
(`7AviUf9n...`) to be fresh within `oracle_interval_secs` (currently 10s on
shadow) at snapshot time; when the shadow price crank lags, `update_state`
fails with `PythOracleOutdated` and the suite reads red. Retry, or bump the
shadow oracle interval.

---

# Titan AMM Integration Template

A reference implementation and test suite for adding AMMs, CLMMs, and proprietary liquidity engines to Titan’s routing layer.

## Overview

Titan aggregates liquidity from heterogeneous venues (AMMs, CLMMs, orderbooks, proprietary pools) under a single unified quoting and routing interface.

This repository provides:

- A compact `TradingVenue` template for describing quote math, token metadata, account loading, and swap instruction shape
- A robust boundary-search engine for computing safe swap-size ranges
- Token metadata utilities, including Token-2022 support
- A caching abstraction for efficient on-chain account loading
- Simulation tests using LiteSVM ensuring off-chain quotes match on-chain execution
- Pricing tests ensuring the reported marginal price is consistent with the quoted output

## On-Chain CPI Template

This repo also includes `program-template/`, an Anchor template for the venue CPI
adapter Titan's router program calls during routed swaps.

Use it to verify the venue's on-chain swap instruction shape against Titan
router account layout and TitanPDA custody:

```bash
cargo check --manifest-path program-template/Cargo.toml
make build-program
```

## Core Components

- `TradingVenue`: implement account parsing, state refresh, token metadata,
  protocol labeling, exact-in quote math, and swap instruction construction.
- `QuoteRequest` / `QuoteResult`: Titan routes `ExactIn` only. All amounts and
  prices use raw atom units, not UI decimal scaling.
- `QuoteResult::price`: report the marginal derivative
  `d(output_atoms) / d(input_atoms)`. It must be positive, non-increasing, and
  consistent with `expected_output`.
- `bounds`: finds safe input ranges from a zero-input-safe `quote()`.
- `TokenInfo`: covers SPL Token, Token-2022, and transfer fee metadata. Do not
  duplicate transfer-fee handling in quote math.
- `AccountsCache`: loads required on-chain accounts with RPC caching.

## Included Tests

The venue must pass the shared suite in `tests/common/mod.rs`, run through
`tests/hylo.rs`.

- Construction and boundaries: deserialization, state loading, token info,
  boundary quotes, and no heap allocation inside `quote()`.
- Simulation: LiteSVM swaps compare on-chain output to off-chain `quote()` at
  boundaries and random samples, while checking accounts, monotonicity, and
  quote speed.
- Pricing: `price` must be positive, non-increasing, and bracket the realized
  average rate: `price(b) <= (f(b) - f(a)) / (b - a) <= price(a)`. The tests
  include atom-rounding slack for truncated integer outputs.

## Running the tests

```bash
make build-program   # build the Titan router program template
make check-structure # fast no-RPC sanity checks
make test-venue     # the Hylo venue suite
make scorecard      # print the integration scorecard only
make dump-programs  # fetch the program binaries the simulation tests load
```

Everything runs cleanly on a fresh clone: the construction, simulation, and
pricing tests need a mainnet RPC endpoint (and, for the simulations, dumped
program binaries), so they **SKIP with an explanation** instead of failing when
those prerequisites are absent. To run them for real:

```bash
export SOLANA_RPC_URL=https://...   # a mainnet RPC endpoint
make build-program                  # rebuild the Titan router program template
make dump-programs                  # one-time: dump the venue programs into programs/
make test-venue
```

`make check-structure` runs unit tests, scorecard assertions, and `Venue` enum
parity checks without requiring RPC.

## Tips for Integrators
1. Always support zero-input quoting
2. Keep your deserialization strictly defensive, never panic
3. Don’t perform I/O, allocate heap memory, or panic inside quote()
4. A quote must average under 1 microsecond (1µs) — see the quoting_speed test
5. Make sure your instruction accounts match the program’s expectations
6. Report a marginal `price` in raw output atoms per raw input atom — positive, non-increasing in size, and consistent with `expected_output`
