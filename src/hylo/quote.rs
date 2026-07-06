use anchor_lang::prelude::Clock;
use anyhow::{Result as AnyhowResult, anyhow};
use fix::prelude::UFix64;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XSOL,
};
use hylo_quotes::protocol_state::{ProtocolAccounts, ProtocolState};
use hylo_quotes::token_operation::TokenOperationExt;
use solana_pubkey::Pubkey;

use super::boxed_err;
use crate::trading_venue::error::TradingVenueError;

/// Probe distance (in raw input atoms) for the finite-difference marginal
/// price: ~0.001 jitoSOL or ~1 hyUSD.
const PRICE_PROBE_DELTA: u64 = 1 << 20;

/// SDK protocol state driving the venue's quotes.
pub struct HyloQuoteState {
  state: ProtocolState<Clock>,
}

impl HyloQuoteState {
  pub fn build(
    protocol_accounts: &ProtocolAccounts,
  ) -> Result<Self, TradingVenueError> {
    let state =
      ProtocolState::try_from(protocol_accounts).map_err(boxed_err)?;
    Ok(HyloQuoteState { state })
  }

  /// Output and marginal price for `amount`, or `None` when the operation
  /// is blocked or the size exceeds what the protocol accepts.
  pub fn quote(
    &self,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
  ) -> Option<(u64, f64)> {
    // Marginal price f'(amount) by backward-difference secant; forward at
    // sizes below the probe distance. One probe end always sits at
    // `amount`, so two evaluations yield both the output and the price.
    let (x0, x1) = if amount >= PRICE_PROBE_DELTA {
      (amount - PRICE_PROBE_DELTA, amount)
    } else {
      (amount, amount + PRICE_PROBE_DELTA)
    };
    let f = |x: u64| self.out(input_mint, output_mint, x);
    match (f(x0), f(x1)) {
      (Ok(y0), Ok(y1)) => {
        let expected_output = if x0 == amount { y0 } else { y1 };
        let price = (y1.saturating_sub(y0)) as f64 / (x1 - x0) as f64;
        Some((expected_output, price))
      }
      _ => None,
    }
  }

  /// Raw-atom output for `amount` atoms of `input_mint`. One arm per routable
  /// pair, mirroring `hylo-router`'s `resolve_route`.
  fn out(
    &self,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
  ) -> AnyhowResult<u64> {
    macro_rules! out {
      ($in:ty, $out:ty) => {
        self
          .state
          .output::<$in, $out>(UFix64::new(amount))?
          .out_amount
          .bits
      };
    }
    let output = match (*input_mint, *output_mint) {
      (JITOSOL::MINT, HYUSD::MINT) => out!(JITOSOL, HYUSD),
      (HYLOSOL::MINT, HYUSD::MINT) => out!(HYLOSOL, HYUSD),
      (JITOSOL::MINT, XSOL::MINT) => out!(JITOSOL, XSOL),
      (HYLOSOL::MINT, XSOL::MINT) => out!(HYLOSOL, XSOL),
      (HYUSD::MINT, JITOSOL::MINT) => out!(HYUSD, JITOSOL),
      (HYUSD::MINT, HYLOSOL::MINT) => out!(HYUSD, HYLOSOL),
      (XSOL::MINT, JITOSOL::MINT) => out!(XSOL, JITOSOL),
      (XSOL::MINT, HYLOSOL::MINT) => out!(XSOL, HYLOSOL),
      (HYUSD::MINT, XSOL::MINT) => out!(HYUSD, XSOL),
      (XSOL::MINT, HYUSD::MINT) => out!(XSOL, HYUSD),
      (JITOSOL::MINT, HYLOSOL::MINT) => out!(JITOSOL, HYLOSOL),
      (HYLOSOL::MINT, JITOSOL::MINT) => out!(HYLOSOL, JITOSOL),
      (CBBTC::MINT, HYUSD::MINT) => out!(CBBTC, HYUSD),
      (CBBTC::MINT, XBTC::MINT) => out!(CBBTC, XBTC),
      (HYUSD::MINT, CBBTC::MINT) => out!(HYUSD, CBBTC),
      (XBTC::MINT, CBBTC::MINT) => out!(XBTC, CBBTC),
      (HYUSD::MINT, XBTC::MINT) => out!(HYUSD, XBTC),
      (XBTC::MINT, HYUSD::MINT) => out!(XBTC, HYUSD),
      (JITOSOL::MINT, USDC::MINT) => out!(JITOSOL, USDC),
      (HYLOSOL::MINT, USDC::MINT) => out!(HYLOSOL, USDC),
      (USDC::MINT, JITOSOL::MINT) => out!(USDC, JITOSOL),
      (USDC::MINT, HYLOSOL::MINT) => out!(USDC, HYLOSOL),
      (CBBTC::MINT, USDC::MINT) => out!(CBBTC, USDC),
      (USDC::MINT, CBBTC::MINT) => out!(USDC, CBBTC),
      (USDC::MINT, HYUSD::MINT) => out!(USDC, HYUSD),
      (HYUSD::MINT, USDC::MINT) => out!(HYUSD, USDC),
      (HYUSD::MINT, SHYUSD::MINT) => out!(HYUSD, SHYUSD),
      (SHYUSD::MINT, HYUSD::MINT) => out!(SHYUSD, HYUSD),
      _ => Err(anyhow!("unsupported pair"))?,
    };
    Ok(output)
  }
}
