use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

use crate::state::HyloOp;

/// Canonical Hylo exchange program id (V1 today, V2 after promotion).
#[cfg(not(feature = "shadow"))]
pub const HYLO_EXCHANGE_PROGRAM_ID: Pubkey =
  pubkey!("HYEXCHtHkBagdStcJCp3xbbb9B7sdMdWXFNj6mdsG4hn");

/// Live V2 "shadow" deployment on mainnet.
#[cfg(feature = "shadow")]
pub const HYLO_EXCHANGE_PROGRAM_ID: Pubkey =
  pubkey!("hyshEX5sNEYhnYPMm8MwMThhBRPuLN3rjoYDbC9esPQ");

const MINT_STABLECOIN_LST: [u8; 8] = [6, 208, 28, 175, 229, 35, 145, 226];
const REDEEM_STABLECOIN_LST: [u8; 8] = [12, 230, 189, 126, 110, 233, 234, 195];
const MINT_LEVERCOIN_LST: [u8; 8] = [184, 75, 99, 100, 153, 64, 243, 68];
const REDEEM_LEVERCOIN_LST: [u8; 8] = [34, 147, 66, 197, 160, 82, 147, 202];
const CONVERT_STABLE_TO_LEVER_LST: [u8; 8] =
  [70, 153, 236, 141, 152, 102, 238, 157];
const CONVERT_LEVER_TO_STABLE_LST: [u8; 8] =
  [203, 251, 143, 181, 230, 117, 64, 207];
const SWAP_LST_TO_LST: [u8; 8] = [105, 39, 155, 165, 242, 2, 235, 99];

fn discriminator(op: HyloOp) -> [u8; 8] {
  match op {
    HyloOp::MintStablecoin => MINT_STABLECOIN_LST,
    HyloOp::RedeemStablecoin => REDEEM_STABLECOIN_LST,
    HyloOp::MintLevercoin => MINT_LEVERCOIN_LST,
    HyloOp::RedeemLevercoin => REDEEM_LEVERCOIN_LST,
    HyloOp::ConvertStableToLever => CONVERT_STABLE_TO_LEVER_LST,
    HyloOp::ConvertLeverToStable => CONVERT_LEVER_TO_STABLE_LST,
    HyloOp::SwapLstToLst => SWAP_LST_TO_LST,
  }
}

/// Build the exchange instruction for one Hylo route leg.
///
/// Data layout: `discriminator (8) || amount_in (u64 LE) || 0u8` — the
/// trailing byte is Borsh for `Option::<SlippageConfig>::None`.
pub fn swap(
  op: HyloOp,
  amount_in: u64,
  account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
  let mut data = Vec::with_capacity(17);
  data.extend_from_slice(&discriminator(op));
  data.extend_from_slice(&amount_in.to_le_bytes());
  data.push(0);
  Ok(vec![Instruction {
    program_id: HYLO_EXCHANGE_PROGRAM_ID,
    accounts: account_metas.to_vec(),
    data,
  }])
}
