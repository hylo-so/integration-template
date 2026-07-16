use anchor_lang::ToAccountMetas;
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
use hylo_idl::pda::BTC_USD_PYTH_FEED;
use hylo_idl::router::client::args::Route;
use hylo_idl::router::instruction_builders::route;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, StakePool, TokenMint, USDC, XBTC,
  XSOL,
};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use crate::trading_venue::QuoteRequest;

/// Builds the `hylo-router` `route` instruction for a swap direction, or
/// `None` for an unroutable pair. One arm per routable pair, mirroring
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
) -> Option<Instruction> {
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
      mint_stablecoin_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (CBBTC::MINT, XBTC::MINT) => {
      mint_levercoin_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (HYUSD::MINT, CBBTC::MINT) => {
      redeem_stablecoin_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (XBTC::MINT, CBBTC::MINT) => {
      redeem_levercoin_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (HYUSD::MINT, XBTC::MINT) => {
      convert_stable_to_lever_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (XBTC::MINT, HYUSD::MINT) => {
      convert_lever_to_stable_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
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
      swap_exo_to_usdc(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
        .to_account_metas(None)
    }
    (USDC::MINT, CBBTC::MINT) => {
      swap_usdc_to_exo(user, CBBTC::MINT, BTC_USD_PYTH_FEED)
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
    _ => None?,
  };
  let args = Route {
    token_a: input_mint,
    token_b: output_mint,
    amount,
    slippage_config: None,
  };
  Some(route(&args, &accounts))
}
