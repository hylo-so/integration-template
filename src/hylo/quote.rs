use anchor_lang::prelude::Clock;
use anyhow::Result as AnyhowResult;
use fix::prelude::UFix64;
use hylo_idl::tokens::{HYLOSOL, HYUSD, JITOSOL, TokenMint, XSOL};
use hylo_quotes::protocol_state::{ProtocolAccounts, ProtocolState};
use hylo_quotes::token_operation::TokenOperationExt;
use solana_pubkey::Pubkey;

use super::{HyloOp, boxed_err};
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
    op: HyloOp,
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
    let f = |x: u64| self.out(op, input_mint, output_mint, x);
    match (f(x0), f(x1)) {
      (Ok(y0), Ok(y1)) => {
        let expected_output = if x0 == amount { y0 } else { y1 };
        let price = (y1.saturating_sub(y0)) as f64 / (x1 - x0) as f64;
        Some((expected_output, price))
      }
      _ => None,
    }
  }

  /// Raw-atom output for `amount` atoms of `input_mint`.
  fn out(
    &self,
    op: HyloOp,
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
    let from_jito = *input_mint == JITOSOL::MINT;
    let to_jito = *output_mint == JITOSOL::MINT;
    let output = match op {
      HyloOp::MintStablecoin if from_jito => out!(JITOSOL, HYUSD),
      HyloOp::MintStablecoin => out!(HYLOSOL, HYUSD),
      HyloOp::RedeemStablecoin if to_jito => out!(HYUSD, JITOSOL),
      HyloOp::RedeemStablecoin => out!(HYUSD, HYLOSOL),
      HyloOp::MintLevercoin if from_jito => out!(JITOSOL, XSOL),
      HyloOp::MintLevercoin => out!(HYLOSOL, XSOL),
      HyloOp::RedeemLevercoin if to_jito => out!(XSOL, JITOSOL),
      HyloOp::RedeemLevercoin => out!(XSOL, HYLOSOL),
      HyloOp::ConvertStableToLever => out!(HYUSD, XSOL),
      HyloOp::ConvertLeverToStable => out!(XSOL, HYUSD),
      HyloOp::SwapLstToLst if from_jito => out!(JITOSOL, HYLOSOL),
      HyloOp::SwapLstToLst => out!(HYLOSOL, JITOSOL),
    };
    Ok(output)
  }
}
