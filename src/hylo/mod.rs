//! Hylo V2 exchange venue.
//!
//! Hylo is not a pool-based AMM: it is an LST-collateralized exchange that
//! mints and redeems the hyUSD stablecoin and the xSOL levercoin against LST
//! collateral (jitoSOL, hyloSOL), converts between hyUSD and xSOL, and swaps
//! LST<->LST through its collateral vaults. All operations are priced off the
//! protocol's global state (NAVs derived from the SOL/USD Pyth feed, LST/SOL
//! stake-pool rates, and collateral-ratio-dependent fee curves) rather than
//! per-pool reserves.
//!
//! The venue's `pool_id` is Hylo's global state account (`pda::HYLO`); its
//! tradable tokens are `[jitoSOL, hyloSOL, hyUSD, xSOL]`, and every ordered
//! pair of those four is a supported direction. Everything protocol-specific
//! comes from the Hylo SDK: account list and state assembly from
//! `hylo_quotes::protocol_state`, quote math from the `TokenOperation` impls
//! on [`ProtocolState`] (the same `hylo-core` code the on-chain program
//! executes), and instructions from `hylo_idl::exchange::instruction_builders`.
//! The only venue-local logic is the mint-pair dispatch, the vault-balance
//! payout caps, and the marginal-price secant.

use ahash::HashSet;
use anchor_lang::AccountDeserialize;
use anchor_lang::prelude::Clock;
use anchor_spl::token::TokenAccount;
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use fix::prelude::UFix64;
use hylo_idl::exchange::client::args;
use hylo_idl::exchange::instruction_builders;
use hylo_idl::pda;
use hylo_idl::tokens::{HYLOSOL, HYUSD, JITOSOL, TokenMint, XSOL};
use hylo_quotes::protocol_state::{ProtocolAccounts, ProtocolState};
use hylo_quotes::token_operation::TokenOperationExt;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

use crate::{
    account_caching::AccountsCache,
    trading_venue::{
        FromAccount, QuoteRequest, QuoteResult, SwapType, TradingVenue,
        error::TradingVenueError,
        protocol::PoolProtocol,
        token_info::{TOKEN_PROGRAM_ID, TokenInfo},
        venue_creation::{ParsedInstruction, PoolCreation},
    },
};

/// Hylo V2 exchange program id. Resolves to the live "shadow" V2 deployment
/// with the default `shadow` feature, or to the canonical id without it.
pub const HYLO_EXCHANGE_PROGRAM_ID: Pubkey = hylo_idl::exchange::ID_CONST;

/// Hylo's global state account — this venue's pool/market id.
pub const HYLO_STATE_ID: Pubkey = pda::HYLO;

/// Anchor discriminator of the exchange's `register_lst` instruction, which is
/// the moment a new LST becomes tradable collateral (new pairs on this venue).
const REGISTER_LST_DISCRIMINATOR: [u8; 8] = [167, 114, 254, 24, 190, 221, 82, 107];

/// Index of the global `hylo` state account in `register_lst`.
const REGISTER_LST_HYLO_INDEX: usize = 1;
/// Index of the newly registered LST mint in `register_lst`.
const REGISTER_LST_MINT_INDEX: usize = 8;

/// Probe distance (in raw input atoms) for the finite-difference marginal
/// price. Large enough that integer truncation of the output amount is
/// negligible against the tests' relative tolerances, small enough that the
/// secant stays local: ~0.001 jitoSOL or ~1 hyUSD.
const PRICE_PROBE_DELTA: u64 = 1 << 20;

/// The Hylo exchange operation behind a (input mint, output mint) direction.
///
/// The on-chain program template's venue adapter receives this through the
/// route `Venue` enum and maps it to the exchange instruction discriminator,
/// so the variant order here is part of the route wire format.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
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
    } else if is_lst(input_mint) && is_lst(output_mint) && input_mint != output_mint {
        Some(HyloOp::SwapLstToLst)
    } else {
        None
    }
}

/// Detect Hylo "pool creations".
///
/// Hylo has one global market (the `hylo` state account); what creates new
/// tradable pairs is `register_lst`, which registers a new LST as collateral.
/// Each registration is reported as a creation of the global market with the
/// new LST plus the two protocol tokens it becomes tradable against.
pub fn parse_pool_creations(instructions: &[ParsedInstruction]) -> Vec<PoolCreation> {
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

/// Accounts fetched on top of [`ProtocolAccounts::pubkeys`]: the LST mints
/// (token metadata) and the LST collateral vaults (payout caps).
fn extra_keys() -> [Pubkey; 4] {
    [
        JITOSOL::MINT,
        HYLOSOL::MINT,
        pda::lst_vault(JITOSOL::MINT),
        pda::lst_vault(HYLOSOL::MINT),
    ]
}

/// Every account needed to rebuild quoting state, in fetch order: the SDK's
/// protocol account list followed by [`extra_keys`].
fn update_keys() -> Vec<Pubkey> {
    let mut keys = ProtocolAccounts::pubkeys();
    keys.extend(extra_keys());
    keys
}

/// SDK protocol state plus the venue-local payout caps.
struct HyloQuoteState {
    state: ProtocolState<Clock>,
    jitosol_vault_balance: u64,
    hylosol_vault_balance: u64,
}

impl HyloQuoteState {
    fn vault_balance(&self, lst_mint: &Pubkey) -> u64 {
        if *lst_mint == JITOSOL::MINT {
            self.jitosol_vault_balance
        } else {
            self.hylosol_vault_balance
        }
    }

    /// Cap an LST payout by the collateral vault's live balance: the on-chain
    /// transfer would fail beyond it, so a larger quote is a liquidity miss.
    fn cap_to_vault(&self, lst_out: u64, lst_mint: &Pubkey) -> anyhow::Result<u64> {
        anyhow::ensure!(
            lst_out <= self.vault_balance(lst_mint),
            "LST payout exceeds vault balance"
        );
        Ok(lst_out)
    }
}

/// Compute the raw-atom output for `amount` atoms of `input_mint` by
/// dispatching the runtime mint pair onto the SDK's statically typed
/// [`hylo_quotes::token_operation::TokenOperation`] impls.
fn hylo_out(
    quote_state: &HyloQuoteState,
    op: HyloOp,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
) -> anyhow::Result<u64> {
    let state = &quote_state.state;
    let from_jito = *input_mint == JITOSOL::MINT;
    let to_jito = *output_mint == JITOSOL::MINT;
    let output = match op {
        HyloOp::MintStablecoin if from_jito => {
            state.output::<JITOSOL, HYUSD>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::MintStablecoin => {
            state.output::<HYLOSOL, HYUSD>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::RedeemStablecoin if to_jito => quote_state.cap_to_vault(
            state.output::<HYUSD, JITOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
        HyloOp::RedeemStablecoin => quote_state.cap_to_vault(
            state.output::<HYUSD, HYLOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
        HyloOp::MintLevercoin if from_jito => {
            state.output::<JITOSOL, XSOL>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::MintLevercoin => {
            state.output::<HYLOSOL, XSOL>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::RedeemLevercoin if to_jito => quote_state.cap_to_vault(
            state.output::<XSOL, JITOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
        HyloOp::RedeemLevercoin => quote_state.cap_to_vault(
            state.output::<XSOL, HYLOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
        HyloOp::ConvertStableToLever => {
            state.output::<HYUSD, XSOL>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::ConvertLeverToStable => {
            state.output::<XSOL, HYUSD>(UFix64::new(amount))?.out_amount.bits
        }
        HyloOp::SwapLstToLst if from_jito => quote_state.cap_to_vault(
            state.output::<JITOSOL, HYLOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
        HyloOp::SwapLstToLst => quote_state.cap_to_vault(
            state.output::<HYLOSOL, JITOSOL>(UFix64::new(amount))?.out_amount.bits,
            output_mint,
        )?,
    };
    Ok(output)
}

/// Hylo V2 exchange venue state.
pub struct HyloVenue {
    /// Address of Hylo's global state account (`pda::HYLO`).
    pub pool_id: Pubkey,
    /// Token metadata for `[jitoSOL, hyloSOL, hyUSD, xSOL]`, populated in
    /// `update_state`.
    token_info: Vec<TokenInfo>,
    /// Accounts that must be fetched to refresh quoting state.
    required_state_pubkeys: HashSet<Pubkey>,
    /// Set to `true` once all required state has been loaded.
    initialized: bool,
    /// Quote state snapshot; shares the on-chain math via the Hylo SDK.
    state: Option<HyloQuoteState>,
}

fn boxed_err<E: Into<Box<dyn std::error::Error>>>(err: E) -> TradingVenueError {
    TradingVenueError::SomethingWentWrong(err.into())
}

impl FromAccount for HyloVenue {
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self, TradingVenueError> {
        use hylo_idl::exchange::accounts::Hylo;
        // Defensive parse: reject anything that is not the Hylo state account.
        let is_hylo_state =
            *pubkey == pda::HYLO && Hylo::try_deserialize(&mut account.data.as_slice()).is_ok();
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
        HYLO_EXCHANGE_PROGRAM_ID
    }

    fn program_dependencies(&self) -> Vec<Pubkey> {
        vec![self.program_id(), TOKEN_PROGRAM_ID]
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

    fn get_required_pubkeys_for_update(&self) -> Result<Vec<Pubkey>, TradingVenueError> {
        Ok(self.required_state_pubkeys.iter().cloned().collect())
    }

    async fn update_state(&mut self, cache: &dyn AccountsCache) -> Result<(), TradingVenueError> {
        let keys = update_keys();
        let accounts = cache.get_accounts(&keys).await?;

        // SDK accounts -> full protocol state (deserialization, oracle
        // validation, exchange contexts) exactly as the SDK's own
        // `RpcStateProvider::fetch_state` does.
        let sdk_count = ProtocolAccounts::expected_count();
        let protocol_accounts =
            ProtocolAccounts::try_from((&keys[..sdk_count], &accounts[..sdk_count]))
                .map_err(boxed_err)?;
        let state = ProtocolState::try_from(&protocol_accounts).map_err(boxed_err)?;

        let get = |index: usize| -> Result<&Account, TradingVenueError> {
            accounts[index]
                .as_ref()
                .ok_or(TradingVenueError::NoAccountFound(keys[index].into()))
        };
        let jitosol_mint_account = get(sdk_count)?;
        let hylosol_mint_account = get(sdk_count + 1)?;
        let parse_vault = |account: &Account, key: &Pubkey| -> Result<u64, TradingVenueError> {
            TokenAccount::try_deserialize(&mut account.data.as_slice())
                .map(|vault| vault.amount)
                .map_err(|_| TradingVenueError::DeserializationFailed((*key).into()))
        };
        let jitosol_vault_balance = parse_vault(get(sdk_count + 2)?, &keys[sdk_count + 2])?;
        let hylosol_vault_balance = parse_vault(get(sdk_count + 3)?, &keys[sdk_count + 3])?;

        // Token metadata: [jitoSOL, hyloSOL, hyUSD, xSOL]. All are classic SPL
        // Token mints; the epoch only matters for Token-2022 transfer fees.
        self.token_info = vec![
            TokenInfo::new(&JITOSOL::MINT, jitosol_mint_account, u64::MAX)?,
            TokenInfo::new(&HYLOSOL::MINT, hylosol_mint_account, u64::MAX)?,
            TokenInfo::new(&HYUSD::MINT, &protocol_accounts.hyusd_mint, u64::MAX)?,
            TokenInfo::new(&XSOL::MINT, &protocol_accounts.xsol_mint, u64::MAX)?,
        ];
        self.state = Some(HyloQuoteState {
            state,
            jitosol_vault_balance,
            hylosol_vault_balance,
        });
        self.initialized = true;
        Ok(())
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, TradingVenueError> {
        if matches!(request.swap_type, SwapType::ExactOut) {
            Err(TradingVenueError::ExactOutNotSupported)
        } else {
            let state = self
                .state
                .as_ref()
                .ok_or(TradingVenueError::NotInitialized(self.pool_id.into()))?;
            let op = hylo_op(&request.input_mint, &request.output_mint)
                .ok_or(TradingVenueError::InvalidMint(request.input_mint.into()))?;

            let f = |amount: u64| {
                hylo_out(state, op, &request.input_mint, &request.output_mint, amount)
            };

            // Marginal price f'(amount) by secant. Backward difference wherever
            // possible: for the concave output function both probe points stay
            // inside the already-validated range, and the secant of a concave f
            // brackets correctly for the mean-value and monotonicity invariants.
            let quoted = f(request.amount).and_then(|expected_output| {
                let (x0, y0, x1, y1) = if request.amount >= PRICE_PROBE_DELTA {
                    let lo = request.amount - PRICE_PROBE_DELTA;
                    (lo, f(lo)?, request.amount, expected_output)
                } else {
                    let hi = request.amount + PRICE_PROBE_DELTA;
                    (request.amount, expected_output, hi, f(hi)?)
                };
                let price = (y1.saturating_sub(y0)) as f64 / (x1 - x0) as f64;
                Ok((expected_output, price))
            });

            // A math error means the operation is blocked (rebalance mode /
            // mint cap / paused) or the size exceeds what the protocol accepts.
            let (expected_output, not_enough_liquidity, price) = match quoted {
                Ok((expected_output, price)) => (expected_output, false, price),
                Err(_) => (0, true, 0.0),
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
        let amount = request.amount;
        let instruction = match op {
            HyloOp::MintStablecoin => instruction_builders::mint_stablecoin_lst(
                user,
                request.input_mint,
                &args::MintStablecoinLst {
                    amount_lst_to_deposit: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::RedeemStablecoin => instruction_builders::redeem_stablecoin_lst(
                user,
                request.output_mint,
                &args::RedeemStablecoinLst {
                    amount_to_redeem: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::MintLevercoin => instruction_builders::mint_levercoin_lst(
                user,
                request.input_mint,
                &args::MintLevercoinLst {
                    amount_lst_to_deposit: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::RedeemLevercoin => instruction_builders::redeem_levercoin_lst(
                user,
                request.output_mint,
                &args::RedeemLevercoinLst {
                    amount_to_redeem: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::ConvertStableToLever => instruction_builders::convert_stable_to_lever_lst(
                user,
                &args::ConvertStableToLeverLst {
                    amount_stablecoin: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::ConvertLeverToStable => instruction_builders::convert_lever_to_stable_lst(
                user,
                &args::ConvertLeverToStableLst {
                    amount_levercoin: amount,
                    slippage_config: None,
                },
            ),
            HyloOp::SwapLstToLst => instruction_builders::swap_lst_to_lst(
                user,
                request.input_mint,
                request.output_mint,
                &args::SwapLstToLst {
                    amount_lst_a: amount,
                    slippage_config: None,
                },
            ),
        };
        Ok(instruction)
    }
}
