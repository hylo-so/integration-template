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
//! pair of those four is a supported direction. Quote math runs on
//! `hylo-core` — the same crate the on-chain program executes — so the
//! off-chain quote matches on-chain execution exactly.

use ahash::HashSet;
use anchor_lang::AccountDeserialize;
use anchor_lang::prelude::Clock;
use anchor_spl::token::{Mint, TokenAccount};
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use fix::prelude::{N9, UFix64};
use hylo_core::asset_swap_config::AssetSwapConfig;
use hylo_core::conversion::{Conversion, SwapConversion};
use hylo_core::exchange_context::{ExchangeContext, LstExchangeContext};
use hylo_core::fees::controller::FeeExtract;
use hylo_core::lst::sol_price::LstSolPrice;
use hylo_core::lst::total_sol_cache::TotalSolCache;
use hylo_core::pyth::OracleConfig;
use hylo_core::rebalance::mode::RebalanceMode;
use hylo_idl::exchange::accounts::{Hylo, LstHeader};
use hylo_idl::exchange::client::args;
use hylo_idl::exchange::instruction_builders;
use hylo_idl::pda;
use hylo_idl::tokens::{HYLOSOL, HYUSD, JITOSOL, TokenMint, XSOL};
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;
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

/// Every account needed to rebuild quoting state, in fetch order:
/// `[hylo, hyUSD mint, xSOL mint, jito header, hylo header, SOL/USD feed,
/// jitoSOL mint, hyloSOL mint, clock, jito vault, hylo vault]`.
fn update_keys() -> [Pubkey; 11] {
    [
        pda::HYLO,
        HYUSD::MINT,
        XSOL::MINT,
        pda::lst_header(JITOSOL::MINT),
        pda::lst_header(HYLOSOL::MINT),
        pda::SOL_USD_PYTH_FEED,
        JITOSOL::MINT,
        HYLOSOL::MINT,
        CLOCK_SYSVAR_ID,
        pda::lst_vault(JITOSOL::MINT),
        pda::lst_vault(HYLOSOL::MINT),
    ]
}

/// `SysvarC1ock11111111111111111111111111111111`
const CLOCK_SYSVAR_ID: Pubkey =
    Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");

/// Per-LST quoting inputs, precomputed in `update_state`.
struct LstParams {
    /// LST/SOL exchange rate from the LST header.
    price: LstSolPrice,
    /// LST/USD conversion built from the SOL/USD oracle + LST/SOL price.
    conversion: Conversion,
    /// Live balance of the protocol's LST collateral vault; every operation
    /// paying out this LST (redeems, LST->LST) is capped by it.
    vault_balance: u64,
}

/// Everything the quote math needs, assembled once per `update_state` from
/// live accounts. Amount-independent values (NAVs, conversions, rebalance
/// mode) are precomputed here so `quote()` only runs the amount-dependent
/// fee and conversion arithmetic — the same `hylo-core` calls the on-chain
/// handlers make, in the same order.
struct HyloQuoteState {
    exchange_context: LstExchangeContext<Clock>,
    jitosol: LstParams,
    hylosol: LstParams,
    lst_swap_config: AssetSwapConfig,
    stablecoin_nav: UFix64<N9>,
    /// `Err` at build time (e.g. zero supply) surfaces per-quote.
    levercoin_mint_nav: Option<UFix64<N9>>,
    levercoin_redeem_nav: Option<UFix64<N9>>,
    swap_conversion: Option<SwapConversion>,
    rebalance_mode: RebalanceMode,
    stablecoin_mint_enabled: bool,
    levercoin_mint_enabled: bool,
    epoch: u64,
}

impl HyloQuoteState {
    fn lst_params(&self, mint: &Pubkey) -> anyhow::Result<&LstParams> {
        if *mint == JITOSOL::MINT {
            Ok(&self.jitosol)
        } else if *mint == HYLOSOL::MINT {
            Ok(&self.hylosol)
        } else {
            Err(anyhow::anyhow!("no LST params for mint {mint}"))
        }
    }
}

/// Cap an LST payout by the collateral vault's live balance: the on-chain
/// transfer would fail beyond it, so a larger quote is a liquidity miss.
fn cap_to_vault(lst_out: u64, params: &LstParams) -> anyhow::Result<u64> {
    anyhow::ensure!(
        lst_out <= params.vault_balance,
        "LST payout exceeds vault balance"
    );
    Ok(lst_out)
}

/// Compute the raw-atom output for `amount` atoms of `input_mint`.
///
/// Each arm mirrors the corresponding on-chain instruction handler: the same
/// `hylo-core` fee, NAV, and conversion calls in the same order, so integer
/// rounding matches execution exactly.
fn hylo_out(
    state: &HyloQuoteState,
    op: HyloOp,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
) -> anyhow::Result<u64> {
    let context = &state.exchange_context;
    let output = match op {
        HyloOp::MintStablecoin => {
            anyhow::ensure!(
                state.stablecoin_mint_enabled,
                "LST stablecoin mint disabled"
            );
            let params = state.lst_params(input_mint)?;
            let FeeExtract {
                amount_remaining, ..
            } = context.stablecoin_mint_fee(&params.price, UFix64::new(amount))?;
            let converted = params
                .conversion
                .lst_to_token(amount_remaining, state.stablecoin_nav)?;
            context.validate_stablecoin_amount(converted)?.bits
        }
        HyloOp::RedeemStablecoin => {
            let params = state.lst_params(output_mint)?;
            let lst_out = params
                .conversion
                .token_to_lst(UFix64::new(amount), state.stablecoin_nav)?;
            cap_to_vault(lst_out.bits, params)?;
            context
                .stablecoin_redeem_fee(&params.price, lst_out)?
                .amount_remaining
                .bits
        }
        HyloOp::MintLevercoin => {
            anyhow::ensure!(
                state.levercoin_mint_enabled,
                "Levercoin mint disabled in current rebalance mode"
            );
            let params = state.lst_params(input_mint)?;
            let FeeExtract {
                amount_remaining, ..
            } = context.levercoin_mint_fee(&params.price, UFix64::new(amount))?;
            let nav = state
                .levercoin_mint_nav
                .ok_or_else(|| anyhow::anyhow!("levercoin mint NAV unavailable"))?;
            params.conversion.lst_to_token(amount_remaining, nav)?.bits
        }
        HyloOp::RedeemLevercoin => {
            anyhow::ensure!(
                state.rebalance_mode != RebalanceMode::Depeg,
                "Levercoin redemption disabled in current rebalance mode"
            );
            let params = state.lst_params(output_mint)?;
            let nav = state
                .levercoin_redeem_nav
                .ok_or_else(|| anyhow::anyhow!("levercoin redeem NAV unavailable"))?;
            let lst_out = params.conversion.token_to_lst(UFix64::new(amount), nav)?;
            cap_to_vault(lst_out.bits, params)?;
            context
                .levercoin_redeem_fee(&params.price, lst_out)?
                .amount_remaining
                .bits
        }
        HyloOp::ConvertStableToLever => {
            anyhow::ensure!(
                state.rebalance_mode != RebalanceMode::Depeg,
                "Swaps are disabled in current rebalance mode"
            );
            let conversion = state
                .swap_conversion
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("swap conversion unavailable"))?;
            let FeeExtract {
                amount_remaining, ..
            } = context.stablecoin_to_levercoin_fee(UFix64::new(amount))?;
            conversion.stable_to_lever(amount_remaining)?.bits
        }
        HyloOp::ConvertLeverToStable => {
            anyhow::ensure!(
                state.rebalance_mode >= RebalanceMode::SellZone1,
                "Swaps are disabled in current rebalance mode"
            );
            let conversion = state
                .swap_conversion
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("swap conversion unavailable"))?;
            let converted = conversion.lever_to_stable(UFix64::new(amount))?;
            let hyusd_total = context.validate_stablecoin_swap_amount(converted)?;
            context
                .levercoin_to_stablecoin_fee(hyusd_total)?
                .amount_remaining
                .bits
        }
        HyloOp::SwapLstToLst => {
            let FeeExtract {
                amount_remaining, ..
            } = state.lst_swap_config.apply_fee(UFix64::new(amount))?;
            let in_params = state.lst_params(input_mint)?;
            let out_params = state.lst_params(output_mint)?;
            let out_amount = in_params.price.convert_lst_amount(
                state.epoch,
                amount_remaining,
                &out_params.price,
            )?;
            cap_to_vault(out_amount.bits, out_params)?;
            out_amount.bits
        }
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
    /// Quote state snapshot; shares the on-chain math via `hylo-core`.
    state: Option<HyloQuoteState>,
}

fn boxed_err<E: Into<Box<dyn std::error::Error>>>(err: E) -> TradingVenueError {
    TradingVenueError::SomethingWentWrong(err.into())
}

impl FromAccount for HyloVenue {
    fn from_account(pubkey: &Pubkey, account: &Account) -> Result<Self, TradingVenueError> {
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

        let get = |index: usize| -> Result<&Account, TradingVenueError> {
            accounts[index]
                .as_ref()
                .ok_or(TradingVenueError::NoAccountFound(keys[index].into()))
        };
        fn parse<A: AccountDeserialize>(
            account: &Account,
            key: &Pubkey,
        ) -> Result<A, TradingVenueError> {
            A::try_deserialize(&mut account.data.as_slice())
                .map_err(|_| TradingVenueError::DeserializationFailed((*key).into()))
        }

        let hylo: Hylo = parse(get(0)?, &keys[0])?;
        let xsol_mint: Mint = parse(get(2)?, &keys[2])?;
        let jitosol_header: LstHeader = parse(get(3)?, &keys[3])?;
        let hylosol_header: LstHeader = parse(get(4)?, &keys[4])?;
        let sol_usd: PriceUpdateV2 = parse(get(5)?, &keys[5])?;
        let clock: Clock = bincode::deserialize(&get(8)?.data)
            .map_err(|_| TradingVenueError::DeserializationFailed(keys[8].into()))?;
        let jitosol_vault: TokenAccount = parse(get(9)?, &keys[9])?;
        let hylosol_vault: TokenAccount = parse(get(10)?, &keys[10])?;

        let epoch = clock.epoch;
        let total_sol_cache: TotalSolCache = hylo.total_sol_cache.into();
        let oracle_config = OracleConfig::new(
            hylo.oracle_interval_secs,
            hylo.oracle_conf_tolerance.try_into().map_err(boxed_err)?,
        );
        let exchange_context = LstExchangeContext::load(
            clock,
            &total_sol_cache,
            hylo.stablecoin_mint_threshold
                .try_into()
                .map_err(boxed_err)?,
            oracle_config,
            hylo.levercoin_fees.into(),
            &sol_usd,
            hylo.virtual_stablecoin.into(),
            Some(&xsol_mint),
            hylo.lst_sell_curve_config.into(),
            hylo.lst_buy_curve_config.into(),
        )
        .map_err(boxed_err)?;
        let lst_swap_config = AssetSwapConfig::new(hylo.lst_swap_fee.into()).map_err(boxed_err)?;

        // Precompute every amount-independent quantity once per refresh.
        let jitosol_price: LstSolPrice = jitosol_header.price_sol.into();
        let hylosol_price: LstSolPrice = hylosol_header.price_sol.into();
        let jitosol = LstParams {
            price: jitosol_price,
            conversion: exchange_context
                .token_conversion(&jitosol_price)
                .map_err(boxed_err)?,
            vault_balance: jitosol_vault.amount,
        };
        let hylosol = LstParams {
            price: hylosol_price,
            conversion: exchange_context
                .token_conversion(&hylosol_price)
                .map_err(boxed_err)?,
            vault_balance: hylosol_vault.amount,
        };
        let stablecoin_nav = exchange_context.stablecoin_nav().map_err(boxed_err)?;
        let levercoin_mint_nav = exchange_context.levercoin_mint_nav().ok();
        let levercoin_redeem_nav = exchange_context.levercoin_redeem_nav().ok();
        let swap_conversion = exchange_context.swap_conversion().ok();
        let rebalance_mode = exchange_context.rebalance_mode();
        let stablecoin_mint_enabled = exchange_context.stablecoin_mint_enabled();
        let levercoin_mint_enabled = exchange_context.levercoin_mint_enabled();

        // Token metadata: [jitoSOL, hyloSOL, hyUSD, xSOL]. All are classic SPL
        // Token mints; the epoch only matters for Token-2022 transfer fees.
        self.token_info = vec![
            TokenInfo::new(&JITOSOL::MINT, get(6)?, u64::MAX)?,
            TokenInfo::new(&HYLOSOL::MINT, get(7)?, u64::MAX)?,
            TokenInfo::new(&HYUSD::MINT, get(1)?, u64::MAX)?,
            TokenInfo::new(&XSOL::MINT, get(2)?, u64::MAX)?,
        ];
        self.state = Some(HyloQuoteState {
            exchange_context,
            jitosol,
            hylosol,
            lst_swap_config,
            stablecoin_nav,
            levercoin_mint_nav,
            levercoin_redeem_nav,
            swap_conversion,
            rebalance_mode,
            stablecoin_mint_enabled,
            levercoin_mint_enabled,
            epoch,
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
