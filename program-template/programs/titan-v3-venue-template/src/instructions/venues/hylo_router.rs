use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

/// Canonical Hylo router program id (V1 today, V2 after promotion).
#[cfg(not(feature = "shadow"))]
pub const HYLO_ROUTER_PROGRAM_ID: Pubkey =
  pubkey!("hyRouTRDAgn65xyyJ3L5c4k5SFmSdr3NxDV8Euzjy3f");

/// Live V2 "shadow" deployment on mainnet.
#[cfg(feature = "shadow")]
pub const HYLO_ROUTER_PROGRAM_ID: Pubkey =
  pubkey!("HyshRo2hkqXGcyCfKU22zhSBPMwokmAnEoxDGeVQz7d");

/// Anchor discriminator of `hylo-router`'s `route` instruction.
const ROUTE: [u8; 8] = [229, 23, 203, 151, 122, 227, 173, 42];

/// Build the `hylo-router` `route` instruction for one Hylo route leg. The
/// router resolves the exchange instruction from the mint pair and forwards
/// the leg's accounts via CPI.
///
/// Data layout:
/// `discriminator (8) || token_a (32) || token_b (32) || amount (u64 LE) ||
/// 0u8` — the trailing byte is Borsh for `Option::<SlippageConfig>::None`.
pub fn swap(
  token_a: Pubkey,
  token_b: Pubkey,
  amount_in: u64,
  account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
  let mut data = Vec::with_capacity(81);
  data.extend_from_slice(&ROUTE);
  data.extend_from_slice(token_a.as_ref());
  data.extend_from_slice(token_b.as_ref());
  data.extend_from_slice(&amount_in.to_le_bytes());
  data.push(0);
  Ok(vec![Instruction {
    program_id: HYLO_ROUTER_PROGRAM_ID,
    accounts: account_metas.to_vec(),
    data,
  }])
}
