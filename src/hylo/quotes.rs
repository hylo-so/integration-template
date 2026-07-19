use fix::prelude::UFix64;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XSOL,
};
use hylo_quotes::prelude::ProtocolState;
use hylo_quotes::token_operation::TokenOperationExt;
use solana_program::clock::Clock;
use solana_pubkey::Pubkey;

/// Pair-erased [`TokenOperationExt::output`] result.
pub struct RuntimeQuote {
  pub out_amount: u64,
  pub marginal_rate: f64,
}

/// Typed [`TokenOperationExt::output`] for one pair.
macro_rules! out {
  ($state:expr, $amount:expr, $in:ty, $out:ty) => {{
    let output = $state.output::<$in, $out>(UFix64::new($amount)).ok()?;
    RuntimeQuote {
      out_amount: output.out_amount.bits,
      marginal_rate: output.marginal_rate,
    }
  }};
}

/// Output amount and marginal rate for a swap, `None` if unroutable or
/// blocked. Arms mirror `swap_instruction`.
#[allow(clippy::too_many_lines)]
pub fn runtime_quote(
  state: &ProtocolState<Clock>,
  input_mint: Pubkey,
  output_mint: Pubkey,
  amount: u64,
) -> Option<RuntimeQuote> {
  let quote = match (input_mint, output_mint) {
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
    (JITOSOL::MINT, USDC::MINT) => out!(state, amount, JITOSOL, USDC),
    (HYLOSOL::MINT, USDC::MINT) => out!(state, amount, HYLOSOL, USDC),
    (USDC::MINT, JITOSOL::MINT) => out!(state, amount, USDC, JITOSOL),
    (USDC::MINT, HYLOSOL::MINT) => out!(state, amount, USDC, HYLOSOL),
    (CBBTC::MINT, USDC::MINT) => out!(state, amount, CBBTC, USDC),
    (USDC::MINT, CBBTC::MINT) => out!(state, amount, USDC, CBBTC),
    (USDC::MINT, HYUSD::MINT) => out!(state, amount, USDC, HYUSD),
    (HYUSD::MINT, USDC::MINT) => out!(state, amount, HYUSD, USDC),
    (HYUSD::MINT, SHYUSD::MINT) => out!(state, amount, HYUSD, SHYUSD),
    (SHYUSD::MINT, HYUSD::MINT) => out!(state, amount, SHYUSD, HYUSD),
    _ => None?,
  };
  Some(quote)
}
