use anchor_lang::prelude::*;

pub const MAX_SWAPS: usize = 12;
pub const MAX_MINTS: usize = 12;

/// Hylo exchange operation for a route leg. Must stay byte-for-byte identical
/// to `HyloOp` in the off-chain route builder (`src/your_venue/mod.rs`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Copy, Eq, Debug)]
pub enum HyloOp {
    MintStablecoin,
    RedeemStablecoin,
    MintLevercoin,
    RedeemLevercoin,
    ConvertStableToLever,
    ConvertLeverToStable,
    SwapLstToLst,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Copy, Eq, Debug)]
pub enum Venue {
    RaydiumAmm,
    /// Hylo V2 exchange; `op` selects the exchange instruction to CPI.
    HyloExchange {
        op: HyloOp,
    },
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Copy, Eq, Debug)]
pub struct SwapSpecInputV2 {
    pub venue: Venue,
    pub from: u8,
    pub to: u8,
    pub weight_nanos: u32,
    pub n_accounts: u8,
}

#[account]
pub struct TitanPda {
    pub bump: u8,
}

impl TitanPda {
    pub const SIZE: usize = 1;
    pub const SEED: &'static [u8] = b"titan_pda";
}
