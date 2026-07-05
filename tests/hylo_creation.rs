use hylo_idl::pda;
use hylo_idl::tokens::{HYLOSOL, HYUSD, JITOSOL, StakePool, TokenMint, XSOL};
use solana_pubkey::Pubkey;

use titan_integration_template::hylo::{
  HYLO_EXCHANGE_PROGRAM_ID, HYLO_STATE_ID, parse_pool_creations,
};
use titan_integration_template::trading_venue::protocol::PoolProtocol;
use titan_integration_template::trading_venue::venue_creation::{
  ParsedInstruction, PoolCreation,
};

/// Anchor discriminator of `register_lst` (from the exchange IDL).
const REGISTER_LST_DISCRIMINATOR: [u8; 8] =
  [167, 114, 254, 24, 190, 221, 82, 107];

/// A `register_lst` fixture for hyloSOL, with the real account order:
/// `[admin, hylo, lst_header, fee_auth, vault_auth, registry_auth, fee_vault,
/// lst_vault, lst_mint, lst_registry, lst_stake_pool_state, ...programs]`.
fn hylosol_registration() -> ParsedInstruction {
  let admin = Pubkey::new_unique();
  let lst_registry = Pubkey::new_unique();
  ParsedInstruction {
    program_id: HYLO_EXCHANGE_PROGRAM_ID,
    accounts: vec![
      admin,
      HYLO_STATE_ID,
      pda::lst_header(HYLOSOL::MINT),
      pda::fee_auth(HYLOSOL::MINT),
      pda::lst_vault_auth(HYLOSOL::MINT),
      pda::LST_REGISTRY_AUTH,
      pda::fee_vault(HYLOSOL::MINT),
      pda::lst_vault(HYLOSOL::MINT),
      HYLOSOL::MINT,
      lst_registry,
      HYLOSOL::POOL_STATE,
    ],
    data: REGISTER_LST_DISCRIMINATOR.to_vec(),
  }
}

fn unrelated_instruction() -> ParsedInstruction {
  ParsedInstruction {
    program_id: HYLO_EXCHANGE_PROGRAM_ID,
    accounts: vec![],
    data: vec![],
  }
}

/// A `mint_stablecoin_lst` call must not register as a creation even though it
/// targets the same program and has enough accounts.
fn mint_instruction() -> ParsedInstruction {
  ParsedInstruction {
    program_id: HYLO_EXCHANGE_PROGRAM_ID,
    accounts: vec![Pubkey::new_unique(); 16],
    data: vec![6, 208, 28, 175, 229, 35, 145, 226],
  }
}

#[test]
fn parses_hylo_lst_registration() {
  let creations = parse_pool_creations(&[hylosol_registration()]);

  assert_eq!(
    creations,
    vec![PoolCreation {
      protocol: PoolProtocol::HyloExchange,
      pool: HYLO_STATE_ID,
      mints: vec![HYLOSOL::MINT, HYUSD::MINT, XSOL::MINT],
    }],
  );
}

#[test]
fn parses_jitosol_registration_alongside_noise() {
  let jitosol_registration = ParsedInstruction {
    accounts: {
      let mut accounts = hylosol_registration().accounts;
      accounts[8] = JITOSOL::MINT;
      accounts
    },
    ..hylosol_registration()
  };
  let creations = parse_pool_creations(&[
    unrelated_instruction(),
    jitosol_registration,
    mint_instruction(),
  ]);

  assert_eq!(creations.len(), 1);
  assert_eq!(creations[0].mints[0], JITOSOL::MINT);
}

#[test]
fn ignores_transactions_without_a_creation() {
  let creations =
    parse_pool_creations(&[unrelated_instruction(), mint_instruction()]);
  assert!(
    creations.is_empty(),
    "a transaction without a pool creation creates no pools, got {creations:?}"
  );
}
