use market_trade_live_lane::gate_compact_stream;
use std::env;
use std::fs;
use std::process;

fn main() {
    let mut args = env::args().skip(1);
    let mut stream_path = None;
    let mut min_game_hours = 1usize;
    let mut require_x15_activation_transition = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream_path = args.next(),
            "--min-game-hours" => {
                let Some(value) = args.next() else {
                    eprintln!("--min-game-hours requires a value");
                    process::exit(2);
                };
                min_game_hours = value.parse().unwrap_or_else(|err| {
                    eprintln!("invalid --min-game-hours {value:?}: {err}");
                    process::exit(2);
                });
            }
            "--require-x15-activation-transition" => {
                require_x15_activation_transition = true;
            }
            "-h" | "--help" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("unknown argument {other:?}");
                print_usage();
                process::exit(2);
            }
        }
    }
    let Some(stream_path) = stream_path else {
        print_usage();
        process::exit(2);
    };
    let raw = fs::read(&stream_path).unwrap_or_else(|err| {
        eprintln!("cannot read {stream_path}: {err}");
        process::exit(2);
    });
    let gate = gate_compact_stream(&raw, min_game_hours);
    let x15_activation_ok = !require_x15_activation_transition
        || (gate.summary.activation_saw_closed_before_active
            && gate.summary.activation_saw_active_after_closed
            && gate.summary.activation_saw_closed_after_active
            && gate.summary.activation_state_transitions >= 2
            && gate.summary.activation_sequence_errors.is_empty());
    println!("ok={}", gate.ok && x15_activation_ok);
    println!("problems={:?}", gate.problems);
    println!("domain_data_records={}", gate.summary.domain_data_records);
    println!("unique_game_hours={}", gate.summary.unique_game_hours);
    println!(
        "skipped_game_hours={}",
        gate.summary.skipped_game_hours.len()
    );
    println!("row_count_total={}", gate.summary.row_count_total);
    println!(
        "observed_contract_rows_total={}",
        gate.summary.observed_contract_rows_total
    );
    println!(
        "omitted_candidate_rows_total={}",
        gate.summary.omitted_candidate_rows_total
    );
    println!(
        "partial_field_rows_total={}",
        gate.summary.partial_field_rows_total
    );
    println!(
        "residual_badge_rows_total={}",
        gate.summary.residual_badge_rows_total
    );
    println!(
        "residual_source_blocked_rows_total={}",
        gate.summary.residual_source_blocked_rows_total
    );
    println!(
        "residual_schema_blocked_rows_total={}",
        gate.summary.residual_schema_blocked_rows_total
    );
    println!(
        "residual_owned_elsewhere_rows_total={}",
        gate.summary.residual_owned_elsewhere_rows_total
    );
    println!(
        "residual_shared_patch_blocked_rows_total={}",
        gate.summary.residual_shared_patch_blocked_rows_total
    );
    println!("detail_rows_total={}", gate.summary.detail_rows_total);
    println!(
        "activation_records={}",
        gate.summary.activation_records
    );
    println!(
        "activation_active_records={}",
        gate.summary.activation_active_records
    );
    println!(
        "activation_closed_records={}",
        gate.summary.activation_closed_records
    );
    println!(
        "activation_state_transitions={}",
        gate.summary.activation_state_transitions
    );
    println!(
        "activation_closed_active_closed={}",
        gate.summary.activation_saw_closed_before_active
            && gate.summary.activation_saw_active_after_closed
            && gate.summary.activation_saw_closed_after_active
    );
    println!(
        "activation_sequence_errors={:?}",
        gate.summary.activation_sequence_errors
    );
    println!(
        "detail_price_valid_rows_total={}",
        gate.summary.detail_price_valid_rows_total
    );
    println!(
        "detail_speed_valid_rows_total={}",
        gate.summary.detail_speed_valid_rows_total
    );
    println!(
        "detail_completed_valid_rows_total={}",
        gate.summary.detail_completed_valid_rows_total
    );
    println!(
        "detail_days_valid_rows_total={}",
        gate.summary.detail_days_valid_rows_total
    );
    println!(
        "detail_delivery_country_valid_rows_total={}",
        gate.summary.detail_delivery_country_valid_rows_total
    );
    println!(
        "detail_convoys_valid_rows_total={}",
        gate.summary.detail_convoys_valid_rows_total
    );
    println!(
        "detail_efficiency_valid_rows_total={}",
        gate.summary.detail_efficiency_valid_rows_total
    );
    println!(
        "detail_delivery_route_valid_rows_total={}",
        gate.summary.detail_delivery_route_valid_rows_total
    );
    println!(
        "detail_progress_valid_rows_total={}",
        gate.summary.detail_progress_valid_rows_total
    );
    println!(
        "lend_lease_mapping_rows_total={}",
        gate.summary.lend_lease_mapping_rows_total
    );
    println!(
        "lend_lease_outgoing_rows_total={}",
        gate.summary.lend_lease_outgoing_rows_total
    );
    println!(
        "lend_lease_incoming_rows_total={}",
        gate.summary.lend_lease_incoming_rows_total
    );
    println!(
        "lend_lease_amount_valid_rows_total={}",
        gate.summary.lend_lease_amount_valid_rows_total
    );
    println!(
        "lend_lease_price_valid_rows_total={}",
        gate.summary.lend_lease_price_valid_rows_total
    );
    println!("parse_errors={:?}", gate.summary.parse_errors);
    process::exit(if gate.ok && x15_activation_ok { 0 } else { 1 });
}

fn print_usage() {
    eprintln!("usage: market_trade_gate --stream <collector_stream.bin> [--min-game-hours <n>] [--require-x15-activation-transition]");
}
