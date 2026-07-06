mod instructions;
mod quote;

use std::error::Error;

use ahash::HashSet;
use anchor_lang::AccountDeserialize;
use async_trait::async_trait;
use hylo_idl::exchange::accounts::Hylo;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XSOL,
};
use hylo_idl::{earn_pool, exchange, pda, router};
use hylo_quotes::protocol_state::ProtocolAccounts;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use self::quote::HyloQuoteState;
use crate::account_caching::AccountsCache;
use crate::trading_venue::error::TradingVenueError;
use crate::trading_venue::protocol::PoolProtocol;
use crate::trading_venue::token_info::{TOKEN_PROGRAM_ID, TokenInfo};
use crate::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};
use crate::trading_venue::{
  FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
};

/// Hylo router program id.
pub const HYLO_ROUTER_PROGRAM_ID: Pubkey = router::ID_CONST;

/// Hylo V2 exchange program id.
pub const HYLO_EXCHANGE_PROGRAM_ID: Pubkey = exchange::ID_CONST;

/// Hylo earn pool program id.
pub const HYLO_EARN_POOL_PROGRAM_ID: Pubkey = earn_pool::ID_CONST;

/// Hylo global state account.
pub const HYLO_STATE_ID: Pubkey = pda::HYLO;

/// Directed pairs served by `hylo-router`'s `resolve_route`.
const ROUTABLE_PAIRS: [(Pubkey, Pubkey); 28] = [
  (JITOSOL::MINT, HYUSD::MINT),
  (HYLOSOL::MINT, HYUSD::MINT),
  (JITOSOL::MINT, XSOL::MINT),
  (HYLOSOL::MINT, XSOL::MINT),
  (HYUSD::MINT, JITOSOL::MINT),
  (HYUSD::MINT, HYLOSOL::MINT),
  (XSOL::MINT, JITOSOL::MINT),
  (XSOL::MINT, HYLOSOL::MINT),
  (HYUSD::MINT, XSOL::MINT),
  (XSOL::MINT, HYUSD::MINT),
  (JITOSOL::MINT, HYLOSOL::MINT),
  (HYLOSOL::MINT, JITOSOL::MINT),
  (CBBTC::MINT, HYUSD::MINT),
  (CBBTC::MINT, XBTC::MINT),
  (HYUSD::MINT, CBBTC::MINT),
  (XBTC::MINT, CBBTC::MINT),
  (HYUSD::MINT, XBTC::MINT),
  (XBTC::MINT, HYUSD::MINT),
  (JITOSOL::MINT, USDC::MINT),
  (HYLOSOL::MINT, USDC::MINT),
  (USDC::MINT, JITOSOL::MINT),
  (USDC::MINT, HYLOSOL::MINT),
  (CBBTC::MINT, USDC::MINT),
  (USDC::MINT, CBBTC::MINT),
  (USDC::MINT, HYUSD::MINT),
  (HYUSD::MINT, USDC::MINT),
  (HYUSD::MINT, SHYUSD::MINT),
  (SHYUSD::MINT, HYUSD::MINT),
];

/// Anchor discriminator of the exchange's `register_lst` instruction.
const REGISTER_LST_DISCRIMINATOR: [u8; 8] =
  [167, 114, 254, 24, 190, 221, 82, 107];

/// Index of the global `hylo` state account in `register_lst`.
const REGISTER_LST_HYLO_INDEX: usize = 1;
/// Index of the newly registered LST mint in `register_lst`.
const REGISTER_LST_MINT_INDEX: usize = 8;

/// Detects `register_lst` instructions, which make a new LST tradable
/// against hyUSD and xSOL.
pub fn parse_pool_creations(
  instructions: &[ParsedInstruction],
) -> Vec<PoolCreation> {
  instructions
    .iter()
    .filter(|ix| {
      ix.program_id == HYLO_EXCHANGE_PROGRAM_ID
        && ix.data.len() >= 8
        && ix.data[..8] == REGISTER_LST_DISCRIMINATOR
        && ix.accounts.len() > REGISTER_LST_MINT_INDEX
    })
    .map(|ix| PoolCreation {
      protocol: PoolProtocol::HyloExchange,
      pool: ix.accounts[REGISTER_LST_HYLO_INDEX],
      mints: vec![
        ix.accounts[REGISTER_LST_MINT_INDEX],
        HYUSD::MINT,
        XSOL::MINT,
      ],
    })
    .collect()
}

/// Accounts needed beyond [`ProtocolAccounts::pubkeys`].
fn extra_keys() -> [Pubkey; 4] {
  [JITOSOL::MINT, HYLOSOL::MINT, USDC::MINT, CBBTC::MINT]
}

/// Every account needed to rebuild quoting state, in fetch order.
fn update_keys() -> Vec<Pubkey> {
  let mut keys = ProtocolAccounts::pubkeys();
  keys.extend(extra_keys());
  keys
}

fn boxed_err<E: Into<Box<dyn Error>>>(err: E) -> TradingVenueError {
  TradingVenueError::SomethingWentWrong(err.into())
}

/// Hylo V2 exchange venue state.
pub struct HyloVenue {
  /// Address of Hylo's global state account (`pda::HYLO`).
  pub pool_id: Pubkey,
  /// Token metadata for
  /// `[jitoSOL, hyloSOL, hyUSD, xSOL, sHYUSD, USDC, cbBTC, xBTC]`, populated
  /// in `update_state`.
  token_info: Vec<TokenInfo>,
  /// Accounts that must be fetched to refresh quoting state.
  required_state_pubkeys: HashSet<Pubkey>,
  /// Set to `true` once all required state has been loaded.
  initialized: bool,
  /// Quote state snapshot.
  state: Option<HyloQuoteState>,
}

impl FromAccount for HyloVenue {
  fn from_account(
    pubkey: &Pubkey,
    account: &Account,
  ) -> Result<Self, TradingVenueError> {
    let is_hylo_state = *pubkey == pda::HYLO
      && Hylo::try_deserialize(&mut account.data.as_slice()).is_ok();
    if is_hylo_state {
      Ok(HyloVenue {
        pool_id: *pubkey,
        token_info: Vec::new(),
        required_state_pubkeys: update_keys().into_iter().collect(),
        initialized: false,
        state: None,
      })
    } else {
      Err(TradingVenueError::FromAccountError((*pubkey).into()))
    }
  }
}

#[async_trait]
impl TradingVenue for HyloVenue {
  fn initialized(&self) -> bool {
    self.initialized
  }

  fn program_id(&self) -> Pubkey {
    HYLO_ROUTER_PROGRAM_ID
  }

  fn program_dependencies(&self) -> Vec<Pubkey> {
    vec![
      self.program_id(),
      HYLO_EXCHANGE_PROGRAM_ID,
      HYLO_EARN_POOL_PROGRAM_ID,
      TOKEN_PROGRAM_ID,
    ]
  }

  fn directions_num(&self) -> Vec<(u8, u8)> {
    let mints: Vec<Pubkey> =
      self.token_info.iter().map(|info| info.pubkey).collect();
    mints
      .iter()
      .zip(0u8..)
      .flat_map(|(input_mint, from)| {
        mints
          .iter()
          .zip(0u8..)
          .filter_map(move |(output_mint, to)| {
            ROUTABLE_PAIRS
              .contains(&(*input_mint, *output_mint))
              .then_some((from, to))
          })
      })
      .collect()
  }

  fn market_id(&self) -> Pubkey {
    self.pool_id
  }

  fn get_token_info(&self) -> &[TokenInfo] {
    &self.token_info
  }

  fn protocol(&self) -> PoolProtocol {
    PoolProtocol::HyloExchange
  }

  fn get_required_pubkeys_for_update(
    &self,
  ) -> Result<Vec<Pubkey>, TradingVenueError> {
    Ok(self.required_state_pubkeys.iter().cloned().collect())
  }

  async fn update_state(
    &mut self,
    cache: &dyn AccountsCache,
  ) -> Result<(), TradingVenueError> {
    let keys = update_keys();
    let accounts = cache.get_accounts(&keys).await?;

    let sdk_count = ProtocolAccounts::expected_count();
    let protocol_accounts =
      ProtocolAccounts::try_from((&keys[..sdk_count], &accounts[..sdk_count]))
        .map_err(boxed_err)?;

    let get = |index: usize| -> Result<&Account, TradingVenueError> {
      accounts[index]
        .as_ref()
        .ok_or(TradingVenueError::NoAccountFound(keys[index].into()))
    };
    let jitosol_mint_account = get(sdk_count)?;
    let hylosol_mint_account = get(sdk_count + 1)?;
    let usdc_mint_account = get(sdk_count + 2)?;
    let cbbtc_mint_account = get(sdk_count + 3)?;

    // The epoch argument only matters for Token-2022 transfer fees.
    self.token_info = vec![
      TokenInfo::new(&JITOSOL::MINT, jitosol_mint_account, u64::MAX)?,
      TokenInfo::new(&HYLOSOL::MINT, hylosol_mint_account, u64::MAX)?,
      TokenInfo::new(&HYUSD::MINT, &protocol_accounts.hyusd_mint, u64::MAX)?,
      TokenInfo::new(&XSOL::MINT, &protocol_accounts.xsol_mint, u64::MAX)?,
      TokenInfo::new(&SHYUSD::MINT, &protocol_accounts.shyusd_mint, u64::MAX)?,
      TokenInfo::new(&USDC::MINT, usdc_mint_account, u64::MAX)?,
      TokenInfo::new(&CBBTC::MINT, cbbtc_mint_account, u64::MAX)?,
      TokenInfo::new(&XBTC::MINT, &protocol_accounts.xbtc_mint, u64::MAX)?,
    ];
    self.state = Some(HyloQuoteState::build(&protocol_accounts)?);
    self.initialized = true;
    Ok(())
  }

  fn quote(
    &self,
    request: QuoteRequest,
  ) -> Result<QuoteResult, TradingVenueError> {
    if matches!(request.swap_type, SwapType::ExactOut) {
      Err(TradingVenueError::ExactOutNotSupported)
    } else {
      let state = self
        .state
        .as_ref()
        .ok_or(TradingVenueError::NotInitialized(self.pool_id.into()))?;

      let quoted =
        state.quote(&request.input_mint, &request.output_mint, request.amount);
      let (expected_output, not_enough_liquidity, price) = match quoted {
        Some((expected_output, price)) => (expected_output, false, price),
        None => (0, true, 0.0),
      };
      Ok(QuoteResult {
        input_mint: request.input_mint,
        output_mint: request.output_mint,
        amount: request.amount,
        expected_output,
        not_enough_liquidity,
        price,
      })
    }
  }

  fn generate_swap_instruction(
    &self,
    request: QuoteRequest,
    user: Pubkey,
  ) -> Result<Instruction, TradingVenueError> {
    instructions::swap_instruction(&request, user)
      .ok_or(TradingVenueError::InvalidMint(request.input_mint.into()))
  }
}
