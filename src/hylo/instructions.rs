use hylo_idl::exchange::account_builders;
use hylo_idl::router::client::args::Route;
use hylo_idl::router::instruction_builders::route;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use super::HyloOp;
use crate::trading_venue::QuoteRequest;

/// Builds the `hylo-router` `route` instruction for a swap direction.
pub fn swap_instruction(
  op: HyloOp,
  request: &QuoteRequest,
  user: Pubkey,
) -> Instruction {
  let route_args = Route {
    token_a: request.input_mint,
    token_b: request.output_mint,
    amount: request.amount,
    slippage_config: None,
  };
  match op {
    HyloOp::MintStablecoin => route(
      &route_args,
      &account_builders::mint_stablecoin_lst(user, request.input_mint),
    ),
    HyloOp::RedeemStablecoin => route(
      &route_args,
      &account_builders::redeem_stablecoin_lst(user, request.output_mint),
    ),
    HyloOp::MintLevercoin => route(
      &route_args,
      &account_builders::mint_levercoin_lst(user, request.input_mint),
    ),
    HyloOp::RedeemLevercoin => route(
      &route_args,
      &account_builders::redeem_levercoin_lst(user, request.output_mint),
    ),
    HyloOp::ConvertStableToLever => route(
      &route_args,
      &account_builders::convert_stable_to_lever_lst(user),
    ),
    HyloOp::ConvertLeverToStable => route(
      &route_args,
      &account_builders::convert_lever_to_stable_lst(user),
    ),
    HyloOp::SwapLstToLst => route(
      &route_args,
      &account_builders::swap_lst_to_lst(
        user,
        request.input_mint,
        request.output_mint,
      ),
    ),
  }
}
