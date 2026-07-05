use anchor_lang::AnchorSerialize;
use titan_integration_template::hylo::HyloOp as RouteBuilderHyloOp;
use titan_integration_template::swap_route::Venue as RouteBuilderVenue;
use titan_v3_venue_template::state::HyloOp as ProgramHyloOp;
use titan_v3_venue_template::state::Venue as ProgramVenue;

#[test]
fn venue_enum_matches_route_builder() {
  let ops = [
    (
      ProgramHyloOp::MintStablecoin,
      RouteBuilderHyloOp::MintStablecoin,
    ),
    (
      ProgramHyloOp::RedeemStablecoin,
      RouteBuilderHyloOp::RedeemStablecoin,
    ),
    (
      ProgramHyloOp::MintLevercoin,
      RouteBuilderHyloOp::MintLevercoin,
    ),
    (
      ProgramHyloOp::RedeemLevercoin,
      RouteBuilderHyloOp::RedeemLevercoin,
    ),
    (
      ProgramHyloOp::ConvertStableToLever,
      RouteBuilderHyloOp::ConvertStableToLever,
    ),
    (
      ProgramHyloOp::ConvertLeverToStable,
      RouteBuilderHyloOp::ConvertLeverToStable,
    ),
    (
      ProgramHyloOp::SwapLstToLst,
      RouteBuilderHyloOp::SwapLstToLst,
    ),
  ];

  let mut cases =
    vec![(ProgramVenue::RaydiumAmm, RouteBuilderVenue::RaydiumAmm)];
  cases.extend(ops.map(|(program_op, route_builder_op)| {
    (
      ProgramVenue::HyloExchange { op: program_op },
      RouteBuilderVenue::HyloExchange {
        op: route_builder_op,
      },
    )
  }));

  for (program, route_builder) in cases {
    let program_bytes = program.try_to_vec().unwrap();
    let route_builder_bytes = route_builder.to_borsh_bytes();
    assert_eq!(
            program_bytes, route_builder_bytes,
            "Venue {program:?} serializes differently between program and route builder — the two \
             enums have drifted; check that variants match in name and order",
        );
  }
}
