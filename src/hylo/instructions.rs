use anchor_lang::ToAccountMetas;
use hylo_core::pyth::PythOracle;
use hylo_idl::earn_pool::account_builders::{deposit, withdraw};
use hylo_idl::exchange::account_builders::{
  convert_lever_to_stable_exo, convert_lever_to_stable_lst,
  convert_stable_to_lever_exo, convert_stable_to_lever_lst, mint_levercoin_exo,
  mint_levercoin_lst, mint_stablecoin_exo, mint_stablecoin_lst,
  mint_stablecoin_usdc, redeem_levercoin_exo, redeem_levercoin_lst,
  redeem_stablecoin_exo, redeem_stablecoin_lst, redeem_stablecoin_usdc,
  swap_exo_to_usdc, swap_lst_to_lst, swap_lst_to_usdc, swap_usdc_to_exo,
  swap_usdc_to_lst,
};
use hylo_idl::router::client::args::Route;
use hylo_idl::router::instruction_builders::route;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYPE, HYUSD, JITOSOL, SHYUSD, StakePool, TokenMint, USDC,
  XBTC, XHYPE, XSOL,
};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use crate::trading_venue::QuoteRequest;
use crate::trading_venue::error::TradingVenueError;

/// Builds the `hylo-router` `route` instruction for a swap direction,
/// erroring on an unroutable pair. One arm per routable pair, mirroring
/// `hylo-router`'s `resolve_route`.
#[allow(clippy::too_many_lines)]
pub fn swap_instruction(
  &QuoteRequest {
    input_mint,
    output_mint,
    amount,
    ..
  }: &QuoteRequest,
  user: Pubkey,
) -> Result<Instruction, TradingVenueError> {
  let accounts = match (input_mint, output_mint) {
    (lst @ (JITOSOL::MINT | HYLOSOL::MINT), HYUSD::MINT) => {
      mint_stablecoin_lst(user, lst).to_account_metas(None)
    }
    (lst @ (JITOSOL::MINT | HYLOSOL::MINT), XSOL::MINT) => {
      mint_levercoin_lst(user, lst).to_account_metas(None)
    }
    (HYUSD::MINT, lst @ (JITOSOL::MINT | HYLOSOL::MINT)) => {
      redeem_stablecoin_lst(user, lst).to_account_metas(None)
    }
    (XSOL::MINT, lst @ (JITOSOL::MINT | HYLOSOL::MINT)) => {
      redeem_levercoin_lst(user, lst).to_account_metas(None)
    }
    (HYUSD::MINT, XSOL::MINT) => {
      convert_stable_to_lever_lst(user).to_account_metas(None)
    }
    (XSOL::MINT, HYUSD::MINT) => {
      convert_lever_to_stable_lst(user).to_account_metas(None)
    }
    (JITOSOL::MINT, HYLOSOL::MINT) => {
      swap_lst_to_lst(user, JITOSOL::MINT, HYLOSOL::MINT).to_account_metas(None)
    }
    (HYLOSOL::MINT, JITOSOL::MINT) => {
      swap_lst_to_lst(user, HYLOSOL::MINT, JITOSOL::MINT).to_account_metas(None)
    }
    (CBBTC::MINT, HYUSD::MINT) => {
      mint_stablecoin_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (CBBTC::MINT, XBTC::MINT) => {
      mint_levercoin_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (HYUSD::MINT, CBBTC::MINT) => {
      redeem_stablecoin_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (XBTC::MINT, CBBTC::MINT) => {
      redeem_levercoin_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (HYUSD::MINT, XBTC::MINT) => {
      convert_stable_to_lever_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (XBTC::MINT, HYUSD::MINT) => {
      convert_lever_to_stable_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (HYPE::MINT, HYUSD::MINT) => {
      mint_stablecoin_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (HYPE::MINT, XHYPE::MINT) => {
      mint_levercoin_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (HYUSD::MINT, HYPE::MINT) => {
      redeem_stablecoin_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (XHYPE::MINT, HYPE::MINT) => {
      redeem_levercoin_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (HYUSD::MINT, XHYPE::MINT) => {
      convert_stable_to_lever_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (XHYPE::MINT, HYUSD::MINT) => {
      convert_lever_to_stable_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (JITOSOL::MINT, USDC::MINT) => {
      swap_lst_to_usdc(user, JITOSOL::MINT, JITOSOL::POOL_STATE)
        .to_account_metas(None)
    }
    (HYLOSOL::MINT, USDC::MINT) => {
      swap_lst_to_usdc(user, HYLOSOL::MINT, HYLOSOL::POOL_STATE)
        .to_account_metas(None)
    }
    (USDC::MINT, JITOSOL::MINT) => {
      swap_usdc_to_lst(user, JITOSOL::MINT, JITOSOL::POOL_STATE)
        .to_account_metas(None)
    }
    (USDC::MINT, HYLOSOL::MINT) => {
      swap_usdc_to_lst(user, HYLOSOL::MINT, HYLOSOL::POOL_STATE)
        .to_account_metas(None)
    }
    (CBBTC::MINT, USDC::MINT) => {
      swap_exo_to_usdc(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (USDC::MINT, CBBTC::MINT) => {
      swap_usdc_to_exo(user, CBBTC::MINT, CBBTC::FEED.address)
        .to_account_metas(None)
    }
    (HYPE::MINT, USDC::MINT) => {
      swap_exo_to_usdc(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (USDC::MINT, HYPE::MINT) => {
      swap_usdc_to_exo(user, HYPE::MINT, HYPE::FEED.address)
        .to_account_metas(None)
    }
    (USDC::MINT, HYUSD::MINT) => {
      mint_stablecoin_usdc(user).to_account_metas(None)
    }
    (HYUSD::MINT, USDC::MINT) => {
      redeem_stablecoin_usdc(user).to_account_metas(None)
    }
    (HYUSD::MINT, SHYUSD::MINT) => deposit(user).to_account_metas(None),
    (SHYUSD::MINT, HYUSD::MINT) => withdraw(user).to_account_metas(None),
    _ => Err(TradingVenueError::InvalidMint(input_mint.into()))?,
  };
  let args = Route {
    token_a: input_mint,
    token_b: output_mint,
    amount,
    slippage_config: None,
  };
  Ok(route(&args, &accounts))
}
