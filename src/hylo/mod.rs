mod error;
mod instructions;
mod quotes;

use self::error::error_chain;
use self::quotes::RuntimeQuote;

use anchor_lang::AccountDeserialize;
use async_trait::async_trait;
use hylo_idl::exchange::accounts::Hylo;
use hylo_idl::tokens::{
  CBBTC, HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, USDC, XBTC, XSOL,
};
use hylo_idl::{earn_pool, exchange, pda, router};
use hylo_quotes::prelude::ProtocolState;
use hylo_quotes::protocol_state::ProtocolAccounts;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_program::clock::Clock;
use solana_pubkey::Pubkey;

use crate::account_caching::AccountsCache;
use crate::trading_venue::error::TradingVenueError;
use crate::trading_venue::protocol::PoolProtocol;
use crate::trading_venue::token_info::{TOKEN_PROGRAM_ID, TokenInfo};
use crate::trading_venue::venue_creation::{ParsedInstruction, PoolCreation};
use crate::trading_venue::{
  FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
};

/// Bidirectional swap pairs supported by Hylo router.
pub const PAIRS: [[Pubkey; 2]; 14] = [
  [JITOSOL::MINT, HYUSD::MINT],
  [JITOSOL::MINT, XSOL::MINT],
  [JITOSOL::MINT, HYLOSOL::MINT],
  [JITOSOL::MINT, USDC::MINT],
  [HYLOSOL::MINT, HYUSD::MINT],
  [HYLOSOL::MINT, XSOL::MINT],
  [HYLOSOL::MINT, USDC::MINT],
  [CBBTC::MINT, HYUSD::MINT],
  [CBBTC::MINT, XBTC::MINT],
  [CBBTC::MINT, USDC::MINT],
  [USDC::MINT, HYUSD::MINT],
  [HYUSD::MINT, XSOL::MINT],
  [HYUSD::MINT, XBTC::MINT],
  [HYUSD::MINT, SHYUSD::MINT],
];

/// Unique mints appearing in [`PAIRS`].
#[must_use]
pub fn pair_mints() -> Vec<Pubkey> {
  PAIRS
    .as_flattened()
    .iter()
    .fold(Vec::new(), |mut acc, mint| {
      if !acc.contains(mint) {
        acc.push(*mint);
      }
      acc
    })
}

#[must_use]
pub fn parse_pool_creations(
  _instructions: &[ParsedInstruction],
) -> Vec<PoolCreation> {
  vec![PoolCreation {
    protocol: PoolProtocol::Hylo,
    pool: pda::HYLO,
    mints: pair_mints(),
  }]
}

/// External mint accounts fetched alongside [`ProtocolAccounts`].
struct ExternalMints<'a> {
  jitosol: &'a Account,
  hylosol: &'a Account,
  usdc: &'a Account,
  cbbtc: &'a Account,
}

impl<'a> ExternalMints<'a> {
  const PUBKEYS: [Pubkey; 4] =
    [JITOSOL::MINT, HYLOSOL::MINT, USDC::MINT, CBBTC::MINT];

  /// Borrows a fetched account list, erroring with the key of the first
  /// missing account.
  fn from_fetched(
    fetched: &'a [Option<Account>],
  ) -> Result<Self, TradingVenueError> {
    if let [Some(jitosol), Some(hylosol), Some(usdc), Some(cbbtc)] = fetched {
      Ok(ExternalMints {
        jitosol,
        hylosol,
        usdc,
        cbbtc,
      })
    } else {
      let (key, _) = Self::PUBKEYS
        .iter()
        .zip(fetched)
        .find(|(_, account)| account.is_none())
        .ok_or(TradingVenueError::FailedToFetchMultipleAccountData)?;
      Err(TradingVenueError::NoAccountFound(key.into()))
    }
  }
}

/// Hylo V2 exchange venue state.
pub struct HyloRouter {
  pub pool_id: Pubkey,
  pub protocol_state: Option<ProtocolState<Clock>>,
  pub token_info: Vec<TokenInfo>,
  pub initialized: bool,
}

impl HyloRouter {
  /// Quoting state; `NotInitialized` before the first `update_state`.
  fn protocol_state(&self) -> Result<&ProtocolState<Clock>, TradingVenueError> {
    self
      .protocol_state
      .as_ref()
      .ok_or(TradingVenueError::NotInitialized(self.pool_id.into()))
  }
}

impl FromAccount for HyloRouter {
  fn from_account(
    pubkey: &Pubkey,
    account: &Account,
  ) -> Result<Self, TradingVenueError> {
    let is_hylo_state = *pubkey == pda::HYLO
      && Hylo::try_deserialize(&mut account.data.as_slice()).is_ok();
    if is_hylo_state {
      Ok(HyloRouter {
        pool_id: *pubkey,
        protocol_state: None,
        token_info: Vec::new(),
        initialized: false,
      })
    } else {
      Err(TradingVenueError::FromAccountError((*pubkey).into()))
    }
  }
}

#[async_trait]
impl TradingVenue for HyloRouter {
  fn initialized(&self) -> bool {
    self.initialized
  }

  fn program_id(&self) -> Pubkey {
    router::ID_CONST
  }

  fn program_dependencies(&self) -> Vec<Pubkey> {
    vec![
      self.program_id(),
      exchange::ID_CONST,
      earn_pool::ID_CONST,
      TOKEN_PROGRAM_ID,
    ]
  }

  fn directions_num(&self) -> Vec<(u8, u8)> {
    let index = |mint: &Pubkey| {
      self
        .token_info
        .iter()
        .position(|info| info.pubkey == *mint)
        .and_then(|i| u8::try_from(i).ok())
    };
    PAIRS
      .iter()
      .filter_map(|[a, b]| {
        let (a, b) = (index(a)?, index(b)?);
        Some([(a, b), (b, a)])
      })
      .flatten()
      .collect()
  }

  /// Protocol-true bounds: the SDK computes the executable input range
  /// from state; no boundary search.
  fn bounds(
    &self,
    tkn_in_ind: u8,
    tkn_out_ind: u8,
  ) -> Result<(u64, u64), TradingVenueError> {
    let input_mint = self.get_token(tkn_in_ind as usize)?.pubkey;
    let output_mint = self.get_token(tkn_out_ind as usize)?.pubkey;
    let state = self.protocol_state()?;
    let lower = state
      .runtime_min_input(input_mint, output_mint)
      .map_err(|e| TradingVenueError::NoQuotableValue(error_chain(e)))?;
    let upper = state
      .runtime_max_input(input_mint, output_mint)
      .map_err(|e| TradingVenueError::NoQuotableValue(error_chain(e)))?;
    Ok((lower, upper))
  }

  fn market_id(&self) -> Pubkey {
    self.pool_id
  }

  fn get_token_info(&self) -> &[TokenInfo] {
    &self.token_info
  }

  fn protocol(&self) -> PoolProtocol {
    PoolProtocol::Hylo
  }

  fn get_required_pubkeys_for_update(
    &self,
  ) -> Result<Vec<Pubkey>, TradingVenueError> {
    Ok(
      ProtocolAccounts::PUBKEYS
        .into_iter()
        .chain(ExternalMints::PUBKEYS)
        .collect(),
    )
  }

  async fn update_state(
    &mut self,
    cache: &dyn AccountsCache,
  ) -> Result<(), TradingVenueError> {
    // Fetch accounts
    let keys = self.get_required_pubkeys_for_update()?;
    let accounts = cache.get_accounts(&keys).await?;

    // Split and validate accounts
    let (protocol, external) = accounts
      .split_at_checked(ProtocolAccounts::PUBKEYS.len())
      .ok_or(TradingVenueError::FailedToFetchMultipleAccountData)?;
    let protocol_accounts = ProtocolAccounts::from_fetched(protocol)
      .map_err(|e| TradingVenueError::NoAccountFound(error_chain(e)))?;
    let ExternalMints {
      jitosol,
      hylosol,
      usdc,
      cbbtc,
    } = ExternalMints::from_fetched(external)?;

    // Epoch information
    let Clock { epoch, .. } =
      bincode::deserialize(&protocol_accounts.clock.data).map_err(|e| {
        TradingVenueError::DeserializationFailed(error_chain(e))
      })?;

    // Hylo state snapshot
    let protocol_state = ProtocolState::try_from(&protocol_accounts)
      .map_err(|e| TradingVenueError::MissingState(error_chain(e)))?;

    // Update state
    self.protocol_state = Some(protocol_state);
    self.token_info = vec![
      TokenInfo::new(&JITOSOL::MINT, jitosol, epoch)?,
      TokenInfo::new(&HYLOSOL::MINT, hylosol, epoch)?,
      TokenInfo::new(&HYUSD::MINT, &protocol_accounts.hyusd_mint, epoch)?,
      TokenInfo::new(&XSOL::MINT, &protocol_accounts.xsol_mint, epoch)?,
      TokenInfo::new(&SHYUSD::MINT, &protocol_accounts.shyusd_mint, epoch)?,
      TokenInfo::new(&USDC::MINT, usdc, epoch)?,
      TokenInfo::new(&CBBTC::MINT, cbbtc, epoch)?,
      TokenInfo::new(&XBTC::MINT, &protocol_accounts.xbtc_mint, epoch)?,
    ];
    self.initialized = true;
    Ok(())
  }

  fn quote(
    &self,
    QuoteRequest {
      input_mint,
      output_mint,
      amount,
      swap_type,
    }: QuoteRequest,
  ) -> Result<QuoteResult, TradingVenueError> {
    if matches!(swap_type, SwapType::ExactOut) {
      Err(TradingVenueError::ExactOutNotSupported)?;
    }
    let quote = quotes::runtime_quote(
      self.protocol_state()?,
      input_mint,
      output_mint,
      amount,
    )?;
    let result = match quote {
      Some(RuntimeQuote {
        expected_output,
        price,
      }) => QuoteResult {
        input_mint,
        output_mint,
        amount,
        expected_output,
        not_enough_liquidity: false,
        price,
      },
      None => QuoteResult {
        input_mint,
        output_mint,
        amount,
        expected_output: 0,
        not_enough_liquidity: true,
        price: 0.0,
      },
    };
    Ok(result)
  }

  fn generate_swap_instruction(
    &self,
    request: QuoteRequest,
    user: Pubkey,
  ) -> Result<Instruction, TradingVenueError> {
    instructions::swap_instruction(&request, user)
  }
}
