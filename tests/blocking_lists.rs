//! Blocking-list witnesses use Redis 7.2.16 recordings, never Firn output.
//! The fixture intentionally fails before server startup while any required
//! expectation is ORACLE-pending; successful Redis recordings are cited at
//! each completed expectation. No Firn validation run is claimed.

use super::*;
use std::collections::BTreeMap;

#[path = "blocking_lists_oracle.rs"]
mod oracle;

pub(super) enum Action {
    Command(&'static str, &'static [&'static str], &'static [u8]),
    Send(&'static str, &'static [&'static str]),
    Pipeline(&'static str, &'static [&'static [&'static str]]),
    Read(&'static str, &'static [u8]),
    WaitBlocked(usize),
    Quiet(&'static str),
    CaptureId(&'static str),
    Resp3(&'static str, &'static [u8]),
    Info(&'static str, &'static str),
    Close(&'static str),
    Aof(&'static [u8]),
    Restart,
}

pub(super) struct Scenario {
    name: &'static str,
    clients: &'static [&'static str],
    aof: bool,
    actions: &'static [Action],
}

fn require_recording(bytes: &[u8], scenario: &str, step: usize) {
    assert_ne!(bytes, b"ORACLE-pending", "{scenario}, step {step}: ORACLE-pending; record this operation on pinned Redis 7.2.16 in CI before accepting blocking-list compatibility");
}

/// Parse whole frames without losing RESP3 push/map types. HELLO's dynamic
/// client ID is not an oracle constant; its recorded protocol field is.
fn wire_frame(stream: &mut TcpStream) -> Vec<u8> {
    let line = reply_line(stream, "blocking-list wire frame");
    let mut bytes = line.as_bytes().to_vec();
    let count = line[1..line.len() - 2].parse::<i64>().unwrap_or(-1);
    match bytes[0] {
        b'$' | b'=' | b'!' if count >= 0 => {
            let mut body = vec![0; count as usize + 2];
            stream.read_exact(&mut body).expect("complete RESP body");
            assert!(body.ends_with(b"\r\n"));
            bytes.extend(body);
        }
        b'*' | b'>' | b'~' | b'%' | b'|' if count >= 0 => {
            let elements = if matches!(bytes[0], b'%' | b'|') { count * 2 } else { count };
            for _ in 0..elements { bytes.extend(wire_frame(stream)); }
            if bytes[0] == b'|' { bytes.extend(wire_frame(stream)); }
        }
        _ => {}
    }
    bytes
}

fn expect_bytes(stream: &mut TcpStream, expected: &[u8], scenario: &str, step: usize) {
    assert!(!expected.is_empty(), "{scenario}, step {step}: a recorded completed wire reply cannot be empty");
    let mut actual = vec![0; expected.len()];
    stream.read_exact(&mut actual).unwrap_or_else(|error|
        panic!("{scenario}, step {step}: reading {} oracle bytes: {error}", expected.len()));
    assert_eq!(actual, expected, "{scenario}, step {step}: Redis 7.2.16 wire bytes");
}

fn send_arguments(stream: &mut TcpStream, arguments: &[&str], identities: &BTreeMap<&str, String>) {
    let resolved: Vec<&str> = arguments.iter().map(|argument| {
        if let Some(name) = argument.strip_prefix("@id:") {
            identities.get(name).unwrap_or_else(|| panic!("uncaptured CLIENT ID for {name}")).as_str()
        } else {
            *argument
        }
    }).collect();
    stream.write_all(&resp(&resolved)).expect("send blocking-list fixture command");
}

fn clients_info(control: &mut TcpStream) -> String {
    control.write_all(&resp(&["INFO", "clients"])).unwrap();
    bulk_reply(control, "blocking-list INFO clients")
}

/// This count is a causal setup barrier, not a sleep that assumes the waiter
/// registered. Selected INFO expectations are checked separately against the
/// recording. Polling has a finite bound and missing fields fail explicitly.
fn wait_blocked(control: &mut TcpStream, count: usize, scenario: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let info = clients_info(control);
        let observed: usize = info_field(&info, "blocked_clients")
            .unwrap_or_else(|| panic!("{scenario}: INFO must expose actual blocked_clients"))
            .parse().expect("numeric blocked_clients");
        if observed == count { return; }
        assert!(Instant::now() < deadline,
            "{scenario}: waiting for {count} registered blocked clients, observed {observed}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn quiet(stream: &mut TcpStream, scenario: &str) {
    stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let observed = stream.peek(&mut [0_u8; 1]);
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    match observed {
        Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
        other => panic!("{scenario}: blocked connection unexpectedly produced output or closed: {other:?}"),
    }
}

fn spawn(program: &CompiledProgram, lease: &support::PortLease, count: usize, aof: bool, drivers: &str) -> ProgramChild {
    let port = lease.port().to_string();
    let count = count.to_string();
    let mut arguments: Vec<&[u8]> = vec![port.as_bytes(), count.as_bytes()];
    if aof { arguments.push(b"blocking.aof"); }
    program.spawn_on_route_with(true, &[("WF_DRIVERS", drivers)], &arguments).with_port(lease)
}

fn run(name: &str) {
    let scenario = oracle::SCENARIOS.iter().find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing blocking-list oracle scenario {name}"));
    assert_eq!(scenario.clients.first(), Some(&"p"), "control client must be first");
    for (step, action) in scenario.actions.iter().enumerate() {
        match action {
            Action::Command(_, _, expected) | Action::Read(_, expected) | Action::Aof(expected) | Action::Resp3(_, expected) =>
                require_recording(expected, name, step),
            Action::Info(_, expected) => require_recording(expected.as_bytes(), name, step),
            _ => {}
        }
    }
    // One driver proves blocked socket ownership permits the producer to run;
    // multiple drivers exercise the same protected registration transitions.
    for drivers in ["1", "4"] {
        let program = CompiledProgram::from_environment();
        let lease = support::lease_port();
        let mut child = Some(spawn(&program, &lease, scenario.clients.len(), scenario.aof, drivers));
        let mut clients: BTreeMap<&str, TcpStream> = scenario.clients.iter().map(|&name| {
            (name, connect_when_ready(lease.port(), child.as_ref().unwrap()))
        }).collect();
        let mut identities = BTreeMap::new();
        for (step, action) in scenario.actions.iter().enumerate() {
            match action {
                Action::Command(client, arguments, expected) => {
                    let stream = clients.get_mut(client).expect("live fixture client");
                    send_arguments(stream, arguments, &identities);
                    expect_bytes(stream, expected, name, step);
                }
                Action::Send(client, arguments) =>
                    send_arguments(clients.get_mut(client).expect("live fixture client"), arguments, &identities),
                Action::Pipeline(client, commands) => {
                    let bytes: Vec<u8> = commands.iter().flat_map(|arguments| resp(arguments)).collect();
                    clients.get_mut(client).unwrap().write_all(&bytes).expect("send one blocking-list pipeline");
                }
                Action::Read(client, expected) =>
                    expect_bytes(clients.get_mut(client).expect("live fixture client"), expected, name, step),
                Action::WaitBlocked(count) => wait_blocked(clients.get_mut("p").unwrap(), *count, name),
                Action::Quiet(client) => quiet(clients.get_mut(client).unwrap(), name),
                Action::CaptureId(client) => {
                    let stream = clients.get_mut(client).unwrap();
                    stream.write_all(&resp(&["CLIENT", "ID"])).unwrap();
                    // This is a dynamic identity used as the next request's
                    // input, not a claimed constant Redis reply expectation.
                    let id = integer_reply(stream, "capture blocking client's identity");
                    identities.insert(*client, id.to_string());
                }
                Action::Resp3(client, expected) => {
                    let stream = clients.get_mut(client).unwrap();
                    stream.write_all(&resp(&["HELLO", "3"])).unwrap();
                    let header = reply_line(stream, "HELLO map header");
                    let pairs: usize = header.strip_prefix('%').and_then(|line| line.strip_suffix("\r\n"))
                        .and_then(|number| number.parse().ok()).expect("HELLO 3 map grammar");
                    let mut protocol = None;
                    for _ in 0..pairs {
                        let field = bulk_reply(stream, "HELLO map field");
                        let value = wire_frame(stream);
                        if field == "proto" { protocol = Some(value); }
                    }
                    assert_eq!(protocol.as_deref(), Some(*expected), "{name}: recorded HELLO protocol field");
                }
                Action::Info(field, expected) => {
                    let info = clients_info(clients.get_mut("p").unwrap());
                    assert_eq!(info_field(&info, field), Some((*expected).to_owned()), "{name}, step {step}: recorded INFO {field}");
                }
                Action::Close(client) => {
                    let stream = clients.remove(client).expect("close live fixture client");
                    stream.shutdown(Shutdown::Both).expect("close blocked fixture socket");
                    drop(stream);
                }
                Action::Aof(expected) => {
                    assert!(scenario.aof, "{name}: AOF inspection needs persistence");
                    let path = aof_incremental_path(program.working_directory(), "blocking.aof");
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        let actual = std::fs::read(&path).expect("read blocking-list incremental AOF");
                        if actual == *expected { break; }
                        assert!(Instant::now() < deadline,
                            "{name}, step {step}: AOF differs from Redis recording: {actual:?}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                Action::Restart => {
                    assert!(scenario.aof, "{name}: replay needs persistence");
                    clients.clear();
                    assert_eq!(finished(child.take().unwrap()).0, 0, "{name}: stop before replay");
                    identities.clear();
                    child = Some(spawn(&program, &lease, 1, true, drivers));
                    clients.insert("p", connect_when_ready(lease.port(), child.as_ref().unwrap()));
                }
            }
        }
        clients.clear();
        assert_eq!(finished(child.take().unwrap()).0, 0, "{name}: clean server exit with {drivers} drivers");
    }
}

#[test]
fn blocking_lists_immediate_all_four_and_move_directions() { run("immediate"); }

#[test]
fn blocking_lists_fifo_whole_variadic_push() { run("fifo"); }

#[test]
fn blocking_lists_multikey_order_and_duplicate_cleanup() { run("multikey"); }

#[test]
fn blocking_lists_ready_keys_rescan_original_operands() { run("ready-rescan"); }

#[test]
fn blocking_lists_exec_and_scripts_defer_waiter_service() { run("execution-unit"); }

#[test]
fn blocking_lists_moves_chain_and_wrong_destination_preserves_source() { run("moves"); }

#[test]
fn blocking_lists_wrongtype_replacement_waits_for_matching_readiness() { run("type-change"); }

#[test]
fn blocking_lists_resp3_subscriber_receives_push_while_blocked() { run("resp3-pubsub"); }

#[test]
fn blocking_lists_disconnect_removes_every_registered_key() { run("disconnect"); }

#[test]
fn blocking_lists_client_unblock_and_info_lifecycle() { run("client-unblock"); }

#[test]
fn blocking_lists_aof_records_delayed_effects_and_replays_state() { run("aof"); }

#[test]
#[ignore = "firn-req-extended-float: finite and noncanonical timeout conversion is not implemented"]
fn blocking_lists_fractional_timeouts_require_oracle_and_whitefoot_gap_resolution() { run("fractional-timeout"); }

#[test]
fn blocking_lists_waiter_pipeline_resumes_after_unblock() { run("waiter-pipeline"); }

#[test]
fn blocking_lists_completion_reuse_between_serial_waits() { run("completion-reuse"); }

#[test]
fn blocking_lists_producer_pipeline_cannot_overtake_existing_waiter() { run("producer-pipeline"); }

#[test]
fn blocking_lists_rename_copy_and_move_producers_signal_readiness() { run("other-producers"); }

#[test]
fn blocking_lists_nested_ready_keys_keep_existing_waiter_fifo() { run("nested-fifo"); }

#[test]
fn blocking_lists_expiry_inside_exec_cancels_intermediate_readiness() { run("expiry-before-ready"); }

#[test]
fn blocking_lists_resp2_subscribers_refuse_all_four_commands() { run("resp2-subscriber-refusal"); }

#[test]
fn blocking_lists_existing_list_producers_finish_before_exec_waiters() { run("existing-list-producers-in-exec"); }

#[test]
fn blocking_lists_promoted_unblock_reply_precedes_targets_later_publication() { run("promoted-unblock-order"); }

#[test]
fn blocking_lists_script_error_drains_and_declared_fallback_preserves_one_increment() { run("script-error-and-declared-fallback"); }

#[test]
fn blocking_lists_argument_errors_precede_timeout_conversion() { run("blocking-argument-precedence"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_blpop_timeout_0_001_resp2() { run("rounded-negative-zero-blpop-timeout--0.001-resp2"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_blpop_timeout_0_0005_resp2() { run("rounded-negative-zero-blpop-timeout--0.0005-resp2"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_blpop_timeout_0_001_resp3() { run("rounded-negative-zero-blpop-timeout--0.001-resp3"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_blpop_timeout_0_0005_resp3() { run("rounded-negative-zero-blpop-timeout--0.0005-resp3"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_brpop_timeout_0_001_resp2() { run("rounded-negative-zero-brpop-timeout--0.001-resp2"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_brpop_timeout_0_0005_resp2() { run("rounded-negative-zero-brpop-timeout--0.0005-resp2"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_brpop_timeout_0_001_resp3() { run("rounded-negative-zero-brpop-timeout--0.001-resp3"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_rounded_negative_zero_brpop_timeout_0_0005_resp3() { run("rounded-negative-zero-brpop-timeout--0.0005-resp3"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_negative_finite_errors_resp2() { run("negative-finite-errors-resp2"); }

#[test]
#[ignore = "firn-req-extended-float: Redis extended timeout rounding and validation are not implemented"]
fn blocking_lists_negative_finite_errors_resp3() { run("negative-finite-errors-resp3"); }
