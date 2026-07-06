mod instructions;
mod quote;

use std::error::Error;

use ahash::HashSet;
use anchor_lang::AccountDeserialize;
use async_trait::async_trait;
use hylo_idl::exchange::accounts::Hylo;
use hylo_idl::tokens::{HYLOSOL, HYUSD, JITOSOL, SHYUSD, TokenMint, XSOL};
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

/// Anchor discriminator of the exchange's `register_lst` instruction.
const REGISTER_LST_DISCRIMINATOR: [u8; 8] =
  [167, 114, 254, 24, 190, 221, 82, 107];

/// Index of the global `hylo` state account in `register_lst`.
const REGISTER_LST_HYLO_INDEX: usize = 1;
/// Index of the newly registered LST mint in `register_lst`.
const REGISTER_LST_MINT_INDEX: usize = 8;

/// The Hylo exchange operation behind a swap direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyloOp {
  /// LST -> hyUSD (`mint_stablecoin_lst`)
  MintStablecoin,
  /// hyUSD -> LST (`redeem_stablecoin_lst`)
  RedeemStablecoin,
  /// LST -> xSOL (`mint_levercoin_lst`)
  MintLevercoin,
  /// xSOL -> LST (`redeem_levercoin_lst`)
  RedeemLevercoin,
  /// hyUSD -> xSOL (`convert_stable_to_lever_lst`)
  ConvertStableToLever,
  /// xSOL -> hyUSD (`convert_lever_to_stable_lst`)
  ConvertLeverToStable,
  /// LST -> LST (`swap_lst_to_lst`)
  SwapLstToLst,
  /// hyUSD -> sHYUSD (earn pool `user_deposit`)
  EarnPoolDeposit,
  /// sHYUSD -> hyUSD (earn pool `user_withdraw`)
  EarnPoolWithdraw,
}

fn is_lst(mint: &Pubkey) -> bool {
  *mint == JITOSOL::MINT || *mint == HYLOSOL::MINT
}

/// Map a swap direction to the Hylo exchange operation serving it, or `None`
/// if the pair is not tradable on this venue.
pub fn hylo_op(input_mint: &Pubkey, output_mint: &Pubkey) -> Option<HyloOp> {
  if is_lst(input_mint) && *output_mint == HYUSD::MINT {
    Some(HyloOp::MintStablecoin)
  } else if *input_mint == HYUSD::MINT && is_lst(output_mint) {
    Some(HyloOp::RedeemStablecoin)
  } else if is_lst(input_mint) && *output_mint == XSOL::MINT {
    Some(HyloOp::MintLevercoin)
  } else if *input_mint == XSOL::MINT && is_lst(output_mint) {
    Some(HyloOp::RedeemLevercoin)
  } else if *input_mint == HYUSD::MINT && *output_mint == XSOL::MINT {
    Some(HyloOp::ConvertStableToLever)
  } else if *input_mint == XSOL::MINT && *output_mint == HYUSD::MINT {
    Some(HyloOp::ConvertLeverToStable)
  } else if is_lst(input_mint)
    && is_lst(output_mint)
    && input_mint != output_mint
  {
    Some(HyloOp::SwapLstToLst)
  } else if *input_mint == HYUSD::MINT && *output_mint == SHYUSD::MINT {
    Some(HyloOp::EarnPoolDeposit)
  } else if *input_mint == SHYUSD::MINT && *output_mint == HYUSD::MINT {
    Some(HyloOp::EarnPoolWithdraw)
  } else {
    None
  }
}

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
fn extra_keys() -> [Pubkey; 2] {
  [JITOSOL::MINT, HYLOSOL::MINT]
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
  /// Token metadata for `[jitoSOL, hyloSOL, hyUSD, xSOL, sHYUSD]`, populated
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
      .enumerate()
      .flat_map(|(from, input_mint)| {
        mints
          .iter()
          .enumerate()
          .filter_map(move |(to, output_mint)| {
            hylo_op(input_mint, output_mint).map(|_| (from as u8, to as u8))
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

    // The epoch argument only matters for Token-2022 transfer fees.
    self.token_info = vec![
      TokenInfo::new(&JITOSOL::MINT, jitosol_mint_account, u64::MAX)?,
      TokenInfo::new(&HYLOSOL::MINT, hylosol_mint_account, u64::MAX)?,
      TokenInfo::new(&HYUSD::MINT, &protocol_accounts.hyusd_mint, u64::MAX)?,
      TokenInfo::new(&XSOL::MINT, &protocol_accounts.xsol_mint, u64::MAX)?,
      TokenInfo::new(&SHYUSD::MINT, &protocol_accounts.shyusd_mint, u64::MAX)?,
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
      let op = hylo_op(&request.input_mint, &request.output_mint)
        .ok_or(TradingVenueError::InvalidMint(request.input_mint.into()))?;

      let quoted = state.quote(
        op,
        &request.input_mint,
        &request.output_mint,
        request.amount,
      );
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
    let op = hylo_op(&request.input_mint, &request.output_mint)
      .ok_or(TradingVenueError::InvalidMint(request.input_mint.into()))?;
    Ok(instructions::swap_instruction(op, &request, user))
  }
}
