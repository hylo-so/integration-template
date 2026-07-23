mod common;

use common::{run_swap_route, RouteConfig};
use hylo_idl::{earn_pool, exchange, pda, router};
use solana_pubkey::Pubkey;
use titan_integration_template::hylo::HyloRouter;

/// Hylo's global state account (`pda::HYLO`) — the venue's market id.
fn pool() -> Pubkey {
  pda::HYLO
}

fn venue_programs() -> Vec<Pubkey> {
  vec![router::ID_CONST, exchange::ID_CONST, earn_pool::ID_CONST]
}

#[tokio::test]
async fn swap_route_both_directions() {
  run_swap_route::<HyloRouter>(RouteConfig {
    pool: pool(),
    venue_programs: venue_programs(),
  })
  .await;
}
