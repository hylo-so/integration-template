use fix::prelude::UFix64;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYPE, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XHYPE,
  XSOL,
};
use hylo_quotes::prelude::ProtocolState;
use hylo_quotes::token_operation::TokenOperationExt;
use solana_program::clock::Clock;
use solana_pubkey::Pubkey;

use super::error::exceeds_liquidity;
use crate::trading_venue::error::TradingVenueError;

/// Pair-erased [`TokenOperationExt::output`] result.
pub struct RuntimeQuote {
  pub expected_output: u64,
  pub price: f64,
}

/// Typed [`TokenOperationExt::output`] for one pair.
macro_rules! out {
  ($state:expr, $amount:expr, $in:ty, $out:ty) => {
    match $state.output::<$in, $out>(UFix64::new($amount)) {
      Ok(output) => Ok(Some(RuntimeQuote {
        expected_output: output.out_amount.bits,
        price: output.marginal_rate,
      })),
      Err(err) if exceeds_liquidity(err) => Ok(None),
      Err(err) => Err(err.into()),
    }
  };
}

/// Output amount and marginal rate for a swap. `Ok(None)` when the size
/// exceeds available liquidity. Arms mirror `swap_instruction`.
#[allow(clippy::too_many_lines)]
pub fn runtime_quote(
  state: &ProtocolState<Clock>,
  input_mint: Pubkey,
  output_mint: Pubkey,
  amount: u64,
) -> Result<Option<RuntimeQuote>, TradingVenueError> {
  match (input_mint, output_mint) {
    (JITOSOL::MINT, HYUSD::MINT) => out!(state, amount, JITOSOL, HYUSD),
    (HYLOSOL::MINT, HYUSD::MINT) => out!(state, amount, HYLOSOL, HYUSD),
    (JITOSOL::MINT, XSOL::MINT) => out!(state, amount, JITOSOL, XSOL),
    (HYLOSOL::MINT, XSOL::MINT) => out!(state, amount, HYLOSOL, XSOL),
    (HYUSD::MINT, JITOSOL::MINT) => out!(state, amount, HYUSD, JITOSOL),
    (HYUSD::MINT, HYLOSOL::MINT) => out!(state, amount, HYUSD, HYLOSOL),
    (XSOL::MINT, JITOSOL::MINT) => out!(state, amount, XSOL, JITOSOL),
    (XSOL::MINT, HYLOSOL::MINT) => out!(state, amount, XSOL, HYLOSOL),
    (HYUSD::MINT, XSOL::MINT) => out!(state, amount, HYUSD, XSOL),
    (XSOL::MINT, HYUSD::MINT) => out!(state, amount, XSOL, HYUSD),
    (JITOSOL::MINT, HYLOSOL::MINT) => out!(state, amount, JITOSOL, HYLOSOL),
    (HYLOSOL::MINT, JITOSOL::MINT) => out!(state, amount, HYLOSOL, JITOSOL),
    (CBBTC::MINT, HYUSD::MINT) => out!(state, amount, CBBTC, HYUSD),
    (CBBTC::MINT, XBTC::MINT) => out!(state, amount, CBBTC, XBTC),
    (HYUSD::MINT, CBBTC::MINT) => out!(state, amount, HYUSD, CBBTC),
    (XBTC::MINT, CBBTC::MINT) => out!(state, amount, XBTC, CBBTC),
    (HYUSD::MINT, XBTC::MINT) => out!(state, amount, HYUSD, XBTC),
    (XBTC::MINT, HYUSD::MINT) => out!(state, amount, XBTC, HYUSD),
    (HYPE::MINT, HYUSD::MINT) => out!(state, amount, HYPE, HYUSD),
    (HYPE::MINT, XHYPE::MINT) => out!(state, amount, HYPE, XHYPE),
    (HYUSD::MINT, HYPE::MINT) => out!(state, amount, HYUSD, HYPE),
    (XHYPE::MINT, HYPE::MINT) => out!(state, amount, XHYPE, HYPE),
    (HYUSD::MINT, XHYPE::MINT) => out!(state, amount, HYUSD, XHYPE),
    (XHYPE::MINT, HYUSD::MINT) => out!(state, amount, XHYPE, HYUSD),
    (JITOSOL::MINT, USDC::MINT) => out!(state, amount, JITOSOL, USDC),
    (HYLOSOL::MINT, USDC::MINT) => out!(state, amount, HYLOSOL, USDC),
    (USDC::MINT, JITOSOL::MINT) => out!(state, amount, USDC, JITOSOL),
    (USDC::MINT, HYLOSOL::MINT) => out!(state, amount, USDC, HYLOSOL),
    (CBBTC::MINT, USDC::MINT) => out!(state, amount, CBBTC, USDC),
    (USDC::MINT, CBBTC::MINT) => out!(state, amount, USDC, CBBTC),
    (HYPE::MINT, USDC::MINT) => out!(state, amount, HYPE, USDC),
    (USDC::MINT, HYPE::MINT) => out!(state, amount, USDC, HYPE),
    (USDC::MINT, HYUSD::MINT) => out!(state, amount, USDC, HYUSD),
    (HYUSD::MINT, USDC::MINT) => out!(state, amount, HYUSD, USDC),
    (HYUSD::MINT, SHYUSD::MINT) => out!(state, amount, HYUSD, SHYUSD),
    (SHYUSD::MINT, HYUSD::MINT) => out!(state, amount, SHYUSD, HYUSD),
    _ => Err(TradingVenueError::InvalidMint(input_mint.into())),
  }
}
