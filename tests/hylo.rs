mod common;

use common::SuiteConfig;
use hylo_idl::{earn_pool, exchange, pda, router};
use solana_pubkey::Pubkey;
use titan_integration_template::hylo::HyloVenue;

// Installs the allocation guard that powers the construction test's
// `assert_no_alloc` checks. The Makefile runs that test under `release-debug`
// so the guard is active; speed tests run under true `--release`.
#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Hylo's global state account (`pda::HYLO`) — the venue's market id.
fn pool() -> Pubkey {
  pda::HYLO
}

fn programs() -> Vec<Pubkey> {
  vec![router::ID_CONST, exchange::ID_CONST, earn_pool::ID_CONST]
}

fn config() -> SuiteConfig {
  SuiteConfig {
    pool: pool(),
    programs: programs(),
  }
}

#[tokio::test]
async fn construction() {
  common::construction::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn zero_input_spot_price() {
  common::zero_input_spot_price::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn bound_simulation() {
  common::bound_simulation::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn random_samples() {
  common::random_samples::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn monotone() {
  common::monotone::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn quoting_speed() {
  common::quoting_speed::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn price_monotone() {
  common::price_monotone::<HyloVenue>(&config()).await;
}

#[tokio::test]
async fn mean_value_theorem() {
  common::mean_value_theorem::<HyloVenue>(&config()).await;
}
