mod common;

use common::{run_swap_route, RouteConfig};
use solana_pubkey::Pubkey;
use titan_integration_template::hylo::{
  HyloVenue, HYLO_EARN_POOL_PROGRAM_ID, HYLO_EXCHANGE_PROGRAM_ID,
  HYLO_ROUTER_PROGRAM_ID, HYLO_STATE_ID,
};

/// Hylo's global state account (`pda::HYLO`) — the venue's market id.
fn pool() -> Pubkey {
  HYLO_STATE_ID
}

fn venue_programs() -> Vec<Pubkey> {
  vec![
    HYLO_ROUTER_PROGRAM_ID,
    HYLO_EXCHANGE_PROGRAM_ID,
    HYLO_EARN_POOL_PROGRAM_ID,
  ]
}

#[tokio::test]
async fn swap_route_both_directions() {
  run_swap_route::<HyloVenue>(RouteConfig {
    pool: pool(),
    venue_programs: venue_programs(),
  })
  .await;
}
