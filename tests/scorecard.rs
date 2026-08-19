//! Integration scorecard — an always-on, no-RPC report.
//!
//! cargo hides passing-test output unless `--nocapture`. To see the report:
//!
//! ```bash
//! make scorecard
//! cargo test --release --test scorecard -- --nocapture
//! ```

use std::fs;
use std::path::PathBuf;

use hylo_idl::{earn_pool, exchange, router};

fn manifest() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Read a repo-relative file, returning "" if it does not exist.
fn read(rel: &str) -> String {
  fs::read_to_string(manifest().join(rel)).unwrap_or_default()
}

/// Whether the program binaries the simulation tests need are dumped.
///
/// Names come from the IDL constants rather than literals so the check
/// follows the `shadow` feature, which swaps every program ID.
fn programs_present() -> bool {
  [exchange::ID_CONST, router::ID_CONST, earn_pool::ID_CONST]
    .iter()
    .all(|id| {
      manifest()
        .join("programs")
        .join(format!("{id}.so"))
        .exists()
    })
}

/// The integration layers, with a check per layer.
const LAYERS: [(&str, &str); 4] = [
  (
    "Creation parser",
    "static pool list (Hylo listings are admin-gated)",
  ),
  (
    "Quote layer",
    "implements quote() returning output + a marginal price",
  ),
  (
    "Program layer",
    "on-chain CPI module + a Venue enum variant",
  ),
  (
    "Route builder",
    "protocol_to_venue mapping + a Venue enum variant",
  ),
];

fn render_subheader(title: &str) -> String {
  let width = 62usize;
  let prefix = format!("  -- {title} ");
  let dashes = "-".repeat(width.saturating_sub(prefix.len()));
  format!("{prefix}{dashes}\n")
}

fn render_layers(title: &str, done: [bool; 4]) -> String {
  let mut s = format!("  {title}\n\n");
  s.push_str(&render_subheader("Layers"));
  s.push_str("  Status  Layer            Detail\n");
  s.push_str(
    "  ------  ---------------  \
     ------------------------------------------------------------\n",
  );
  for (i, (layer, desc)) in LAYERS.iter().enumerate() {
    s.push_str(&format!(
      "  {:<6}  {:<15}  {}\n",
      if done[i] { "[x]" } else { "[ ]" },
      layer,
      desc
    ));
  }
  s
}

fn render_simulation() -> String {
  let (status, detail) =
    if std::env::var("SOLANA_RPC_URL").is_ok() && programs_present() {
      ("ENABLED", "SOLANA_RPC_URL set and program dumps present")
    } else {
      ("SKIPPED", "set SOLANA_RPC_URL and run `make dump-programs`")
    };

  format!(
    "\n{}  Status    Detail\n  --------  \
     ------------------------------------------------------------\n  \
     {status:<8}  {detail}\n",
    render_subheader("Simulation")
  )
}

#[test]
fn integration_scorecard() {
  const PROGRAM_SRC: &str =
    "program-template/programs/titan-v3-venue-template/src";

  let hylo_venue = read("src/hylo/mod.rs");
  let hylo_creation = read("tests/hylo_creation.rs");
  let hylo_cpi =
    read(&format!("{PROGRAM_SRC}/instructions/venues/hylo_router.rs"));
  let swap_route = read("src/swap_route/mod.rs");
  let state = read(&format!("{PROGRAM_SRC}/state.rs"));

  let done = [
    hylo_venue.contains("fn parse_pool_creations")
      && hylo_creation.contains("static_pool_list"),
    hylo_venue.contains("fn quote"),
    !hylo_cpi.is_empty() && state.contains("Hylo"),
    swap_route.contains("PoolProtocol::Hylo")
      && swap_route.contains("Venue::Hylo"),
  ];

  let mut report = String::new();
  report.push_str(
    "\n================ Titan integration scorecard ================\n\n",
  );
  report.push_str(&render_layers("Hylo venue:", done));
  report.push_str(&render_simulation());
  report.push_str(
    "=============================================================\n",
  );
  println!("{report}");

  assert!(
    done.iter().all(|d| *d),
    "integration layer missing — check the [ ] items above",
  );
}
