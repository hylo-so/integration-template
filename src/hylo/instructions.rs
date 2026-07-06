use hylo_idl::earn_pool::account_builders as earn_pool_builders;
use hylo_idl::exchange::account_builders;
use hylo_idl::pda;
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
pub fn swap_instruction(
  request: &QuoteRequest,
  user: Pubkey,
) -> Option<Instruction> {
  let args = Route {
    token_a: request.input_mint,
    token_b: request.output_mint,
    amount: request.amount,
    slippage_config: None,
  };
  let lst = request.input_mint;
  let instruction = match (request.input_mint, request.output_mint) {
    (JITOSOL::MINT | HYLOSOL::MINT, HYUSD::MINT) => {
      route(&args, &account_builders::mint_stablecoin_lst(user, lst))
    }
    (JITOSOL::MINT | HYLOSOL::MINT, XSOL::MINT) => {
      route(&args, &account_builders::mint_levercoin_lst(user, lst))
    }
    (HYUSD::MINT, lst @ (JITOSOL::MINT | HYLOSOL::MINT)) => {
      route(&args, &account_builders::redeem_stablecoin_lst(user, lst))
    }
    (XSOL::MINT, lst @ (JITOSOL::MINT | HYLOSOL::MINT)) => {
      route(&args, &account_builders::redeem_levercoin_lst(user, lst))
    }
    (HYUSD::MINT, XSOL::MINT) => {
      route(&args, &account_builders::convert_stable_to_lever_lst(user))
    }
    (XSOL::MINT, HYUSD::MINT) => {
      route(&args, &account_builders::convert_lever_to_stable_lst(user))
    }
    (JITOSOL::MINT, HYLOSOL::MINT) => route(
      &args,
      &account_builders::swap_lst_to_lst(user, JITOSOL::MINT, HYLOSOL::MINT),
    ),
    (HYLOSOL::MINT, JITOSOL::MINT) => route(
      &args,
      &account_builders::swap_lst_to_lst(user, HYLOSOL::MINT, JITOSOL::MINT),
    ),
    (CBBTC::MINT, HYUSD::MINT) => route(
      &args,
      &account_builders::mint_stablecoin_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (CBBTC::MINT, XBTC::MINT) => route(
      &args,
      &account_builders::mint_levercoin_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (HYUSD::MINT, CBBTC::MINT) => route(
      &args,
      &account_builders::redeem_stablecoin_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (XBTC::MINT, CBBTC::MINT) => route(
      &args,
      &account_builders::redeem_levercoin_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (HYUSD::MINT, XBTC::MINT) => route(
      &args,
      &account_builders::convert_stable_to_lever_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (XBTC::MINT, HYUSD::MINT) => route(
      &args,
      &account_builders::convert_lever_to_stable_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (JITOSOL::MINT, USDC::MINT) => route(
      &args,
      &account_builders::swap_lst_to_usdc(
        user,
        JITOSOL::MINT,
        JITOSOL::POOL_STATE,
      ),
    ),
    (HYLOSOL::MINT, USDC::MINT) => route(
      &args,
      &account_builders::swap_lst_to_usdc(
        user,
        HYLOSOL::MINT,
        HYLOSOL::POOL_STATE,
      ),
    ),
    (USDC::MINT, JITOSOL::MINT) => route(
      &args,
      &account_builders::swap_usdc_to_lst(
        user,
        JITOSOL::MINT,
        JITOSOL::POOL_STATE,
      ),
    ),
    (USDC::MINT, HYLOSOL::MINT) => route(
      &args,
      &account_builders::swap_usdc_to_lst(
        user,
        HYLOSOL::MINT,
        HYLOSOL::POOL_STATE,
      ),
    ),
    (CBBTC::MINT, USDC::MINT) => route(
      &args,
      &account_builders::swap_exo_to_usdc(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (USDC::MINT, CBBTC::MINT) => route(
      &args,
      &account_builders::swap_usdc_to_exo(
        user,
        CBBTC::MINT,
        pda::BTC_USD_PYTH_FEED,
      ),
    ),
    (USDC::MINT, HYUSD::MINT) => {
      route(&args, &account_builders::mint_stablecoin_usdc(user))
    }
    (HYUSD::MINT, USDC::MINT) => {
      route(&args, &account_builders::redeem_stablecoin_usdc(user))
    }
    (HYUSD::MINT, SHYUSD::MINT) => {
      route(&args, &earn_pool_builders::deposit(user))
    }
    (SHYUSD::MINT, HYUSD::MINT) => {
      route(&args, &earn_pool_builders::withdraw(user))
    }
    _ => None?,
  };
  Some(instruction)
}
