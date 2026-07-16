use hylo_idl::pda;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XSOL,
};
use titan_integration_template::hylo::parse_pool_creations;
use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::PoolCreation;

/// Hylo listings are admin-gated, so the pool list is static: one pool (the
/// global state account) over every routable mint, regardless of input.
#[test]
fn static_pool_list() {
  let expected = vec![PoolCreation {
    protocol: PoolProtocol::HyloExchange,
    pool: pda::HYLO,
    mints: vec![
      JITOSOL::MINT,
      HYUSD::MINT,
      XSOL::MINT,
      HYLOSOL::MINT,
      USDC::MINT,
      CBBTC::MINT,
      XBTC::MINT,
      SHYUSD::MINT,
    ],
  }];
  assert_eq!(parse_pool_creations(&[]), expected);
}
