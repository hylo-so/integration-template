use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::InstructionData;
use hylo_idl::router;
use hylo_idl::router::client::args::Route;

/// Build the `hylo-router` `route` instruction for one Hylo route leg. The
/// router resolves the exchange instruction from the mint pair and forwards
/// the leg's accounts via CPI. Program id, discriminator, and argument
/// layout all come from the router IDL.
pub fn swap(
  token_a: Pubkey,
  token_b: Pubkey,
  amount_in: u64,
  account_metas: &[AccountMeta],
) -> Result<Vec<Instruction>> {
  let data = Route {
    token_a,
    token_b,
    amount: amount_in,
    slippage_config: None,
  }
  .data();
  Ok(vec![Instruction {
    program_id: router::ID_CONST,
    accounts: account_metas.to_vec(),
    data,
  }])
}
