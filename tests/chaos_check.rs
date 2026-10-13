//! Independent journal/transaction witness for the chaos workload.
//!
//! Redis APPEND returns the resulting byte length. MULTI/EXEC executes each
//! queued command in one uninterrupted sequence, SET replaces a value, and
//! INCR adds one to a missing (initially zero) or integer value. Those command
//! semantics, client histories, and unique tokens are the oracle; snapshots
//! and AOF bytes are never used to generate an expected successful history.

use super::append_history::{Append, check_appends, SHARED_KEYS, TOKEN_BYTES};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub const KEYS: usize = SHARED_KEYS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind { Append, Set, Incr, Both }

impl Kind {
    fn sets(self) -> bool { matches!(self, Self::Set | Self::Both) }
    fn increments(self) -> bool { matches!(self, Self::Incr | Self::Both) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Applied { length: i64, counter: Option<i64> },
    Unknown,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub key: usize,
    pub token: String,
    pub kind: Kind,
    pub invoked: Instant,
    pub completed: Instant,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    pub logs: Vec<String>,
    pub sets: Vec<Option<String>>,
    pub counters: Vec<Option<i64>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Findings {
    pub lost: Vec<String>,
    pub inconsistent: Vec<String>,
}

/// Each group begins empty and is never deleted. An unknown request can be
/// absent or present exactly once; its effect interval stays open until the
/// quiescent checkpoint. Successful replies keep their own ranks and response
/// times. SET acknowledgments are validated by the protocol reader, since an
/// Applied event represents an entirely successful APPEND or EXEC reply.
pub fn check(events: &[Event], state: &State, checkpoint: Instant) -> Findings {
    let mut findings = Findings::default();
    if state.logs.len() != KEYS || state.sets.len() != KEYS || state.counters.len() != KEYS {
        findings.inconsistent.push("wrong number of snapshot key groups".into());
        return findings;
    }
    let mut submitted = HashMap::new();
    for event in events {
        if event.key >= KEYS || event.token.len() != TOKEN_BYTES || !event.token.is_ascii()
            || event.completed < event.invoked || event.completed > checkpoint {
            findings.inconsistent.push(format!("invalid request token/key/interval: {:?}", event.token));
            continue;
        }
        if submitted.insert(event.token.as_str(), event).is_some() {
            findings.inconsistent.push(format!("duplicate submitted token {}", event.token));
        }
        if let Outcome::Applied { length, counter } = event.outcome {
            if length <= 0 || length % TOKEN_BYTES as i64 != 0
                || counter.is_some() != event.kind.increments() {
                findings.inconsistent.push(format!("invalid acknowledgment fields for {}", event.token));
            }
        }
    }
    // Malformed input must fail before it can become a fabricated witness.
    if !findings.inconsistent.is_empty() { return findings; }

    let mut witness = Vec::new();
    for key in 0..KEYS {
        let log = &state.logs[key];
        let mut present = HashSet::new();
        let mut expected_set = None;
        let mut expected_counter = None;
        if log.len() % TOKEN_BYTES != 0 {
            findings.inconsistent.push(format!("key {key}: incomplete journal token"));
        }
        // Slice bytes rather than String to diagnose malformed UTF-8 token
        // boundaries without panicking on an untrusted server snapshot.
        for (index, bytes) in log.as_bytes().chunks(TOKEN_BYTES).enumerate() {
            let token = match std::str::from_utf8(bytes) {
                Ok(token) if token.len() == TOKEN_BYTES && token.is_ascii() => token,
                _ => {
                    findings.inconsistent.push(format!("key {key}: malformed journal token at rank {}", index + 1));
                    continue;
                }
            };
            if !present.insert(token) {
                findings.inconsistent.push(format!("key {key}: duplicated effect for {token}"));
                continue;
            }
            let event = match submitted.get(token) {
                Some(event) if event.key == key => *event,
                _ => {
                    findings.inconsistent.push(format!("key {key}: unknown or wrong-key journal token {token}"));
                    continue;
                }
            };
            if event.outcome == Outcome::Rejected {
                findings.inconsistent.push(format!("key {key}: rejected request {token} has an effect"));
                continue;
            }
            let rank_length = match (index + 1).checked_mul(TOKEN_BYTES).and_then(|n| i64::try_from(n).ok()) {
                Some(length) => length,
                None => {
                    findings.inconsistent.push(format!("key {key}: journal rank exceeds integer range"));
                    continue;
                }
            };
            let (length, completed) = match event.outcome {
                Outcome::Applied { length, .. } => (length, event.completed),
                Outcome::Unknown => (rank_length, checkpoint),
                Outcome::Rejected => unreachable!(),
            };
            witness.push(Append { key, token: token.into(), length, invoked: event.invoked, completed });
            if event.kind.sets() { expected_set = Some(token.to_owned()); }
            if event.kind.increments() {
                expected_counter = Some(expected_counter.unwrap_or(0_i64) + 1);
            }
            if let Outcome::Applied { counter, .. } = event.outcome {
                if event.kind.increments() && counter != expected_counter {
                    findings.inconsistent.push(format!("key {key}: wrong INCR reply for {token}: expected {expected_counter:?}, got {counter:?}"));
                }
            }
        }
        for event in events.iter().filter(|event| event.key == key) {
            if matches!(event.outcome, Outcome::Applied { .. }) && !present.contains(event.token.as_str()) {
                findings.lost.push(format!("key {key}: missing acknowledged token {}", event.token));
            }
        }
        if state.sets[key] != expected_set {
            findings.inconsistent.push(format!("key {key}: final SET differs from journal model: expected {expected_set:?}, got {:?}", state.sets[key]));
        }
        if state.counters[key] != expected_counter {
            findings.inconsistent.push(format!("key {key}: final INCR differs from journal model: expected {expected_counter:?}, got {:?}", state.counters[key]));
        }
    }
    if let Err(error) = check_appends(&witness, &state.logs) {
        findings.inconsistent.push(error);
    }
    findings
}

#[cfg(test)]
mod self_tests {
    use super::*;
    use std::time::Duration;

    fn empty() -> State {
        State { logs: vec![String::new(); KEYS], sets: vec![None; KEYS], counters: vec![None; KEYS] }
    }

    fn event(begin: Instant, id: u64, kind: Kind, rank: i64, counter: Option<i64>) -> Event {
        Event { key: 0, token: format!("{id:016x}"), kind,
            invoked: begin + Duration::from_millis(id * 2),
            completed: begin + Duration::from_millis(id * 2 + 1),
            outcome: Outcome::Applied { length: rank * TOKEN_BYTES as i64, counter } }
    }

    fn fixture() -> (Vec<Event>, State, Instant) {
        let begin = Instant::now();
        let events = vec![
            event(begin, 1, Kind::Append, 1, None),
            event(begin, 2, Kind::Set, 2, None),
            event(begin, 3, Kind::Incr, 3, Some(1)),
            event(begin, 4, Kind::Both, 4, Some(2)),
        ];
        let mut state = empty();
        state.logs[0] = events.iter().map(|event| event.token.as_str()).collect();
        state.sets[0] = Some(events[3].token.clone());
        state.counters[0] = Some(2);
        (events, state, begin + Duration::from_secs(1))
    }

    #[test]
    fn chaos_checker_accepts_serial_overlapping_and_independent_key_histories() {
        let (mut events, mut state, checkpoint) = fixture();
        assert_eq!(check(&events, &state, checkpoint), Findings::default());
        events[1].invoked = events[0].invoked;
        events[0].completed = events[1].completed + Duration::from_millis(1);
        assert_eq!(check(&events, &state, checkpoint), Findings::default(), "overlap allows replies out of rank order");
        let mut other = events[0].clone();
        other.key = 1;
        other.token = "0000000000000010".into();
        state.logs[1] = other.token.clone();
        events.push(other);
        assert_eq!(check(&events, &state, checkpoint), Findings::default(), "independent keys compose");
    }

    #[test]
    fn chaos_checker_rejects_lost_acknowledged_writes() {
        let (events, mut state, checkpoint) = fixture();
        state.logs[0].truncate(3 * TOKEN_BYTES);
        state.sets[0] = Some(events[1].token.clone());
        state.counters[0] = Some(1);
        let result = check(&events, &state, checkpoint);
        assert_eq!(result.lost.len(), 1, "an internally consistent surviving prefix still lost an acknowledgment");
        assert!(result.lost[0].contains(&events[3].token));
    }

    #[test]
    fn chaos_checker_rejects_duplicate_effects_and_duplicate_reply_ranks() {
        let (mut events, mut state, checkpoint) = fixture();
        state.logs[0].push_str(&events[0].token);
        assert!(check(&events, &state, checkpoint).inconsistent.iter().any(|s| s.contains("duplicated effect")));
        state.logs[0].truncate(4 * TOKEN_BYTES);
        events[1].outcome = events[0].outcome.clone();
        assert!(check(&events, &state, checkpoint).inconsistent.iter().any(|s| s.contains("reply rank")));
    }

    #[test]
    fn chaos_checker_rejects_impossible_real_time_order() {
        let (mut events, state, checkpoint) = fixture();
        events[0].invoked = events[1].completed + Duration::from_millis(1);
        events[0].completed = events[0].invoked;
        assert!(check(&events, &state, checkpoint).inconsistent.iter().any(|s| s.contains("real-time")));
    }

    #[test]
    fn chaos_checker_rejects_partial_transactions_and_wrong_final_fields() {
        let (events, state, checkpoint) = fixture();
        let mut partial = state.clone();
        partial.counters[0] = Some(1);
        assert!(check(&events, &partial, checkpoint).inconsistent.iter().any(|s| s.contains("final INCR")), "journal and SET survived but INCR did not");
        let mut partial = state.clone();
        partial.sets[0] = Some(events[1].token.clone());
        assert!(check(&events, &partial, checkpoint).inconsistent.iter().any(|s| s.contains("final SET")), "journal and INCR survived but SET did not");
        let mut partial = state;
        partial.logs[0].truncate(3 * TOKEN_BYTES);
        let result = check(&events, &partial, checkpoint);
        assert!(!result.lost.is_empty());
        assert!(result.inconsistent.iter().any(|s| s.contains("final SET")), "SET survived without its journal effect");
    }

    #[test]
    fn chaos_checker_rejects_wrong_counter_reply_even_when_snapshot_is_correct() {
        let (mut events, state, checkpoint) = fixture();
        events[2].outcome = Outcome::Applied { length: 48, counter: Some(7) };
        assert!(check(&events, &state, checkpoint).inconsistent.iter().any(|s| s.contains("wrong INCR reply")));
    }

    #[test]
    fn chaos_checker_accepts_unknown_once_or_absent_and_keeps_its_interval_open() {
        let begin = Instant::now();
        let checkpoint = begin + Duration::from_secs(1);
        let mut unknown = event(begin, 1, Kind::Both, 1, Some(1));
        unknown.outcome = Outcome::Unknown;
        let mut state = empty();
        assert_eq!(check(&[unknown.clone()], &state, checkpoint), Findings::default(), "an unread request may have no effect");
        state.logs[0] = unknown.token.clone();
        state.sets[0] = Some(unknown.token.clone());
        state.counters[0] = Some(1);
        assert_eq!(check(&[unknown.clone()], &state, checkpoint), Findings::default(), "an unread EXEC may have a complete effect");
        let mut prior = event(begin, 2, Kind::Append, 1, None);
        unknown.invoked = prior.completed;
        unknown.completed = unknown.invoked;
        state.logs[0] = format!("{}{}", prior.token, unknown.token);
        assert_eq!(check(&[prior.clone(), unknown.clone()], &state, checkpoint), Findings::default());
        // Completing Unknown when the connection closes would incorrectly
        // reject this later-applied operation after another request started.
        unknown.invoked = begin;
        unknown.completed = begin;
        prior.invoked = begin + Duration::from_millis(10);
        prior.completed = begin + Duration::from_millis(11);
        assert_eq!(check(&[prior, unknown], &state, checkpoint), Findings::default());
    }

    #[test]
    fn chaos_checker_rejects_unknown_partial_transaction_and_refused_effect() {
        let begin = Instant::now();
        let checkpoint = begin + Duration::from_secs(1);
        let mut request = event(begin, 1, Kind::Both, 1, Some(1));
        request.outcome = Outcome::Unknown;
        let mut state = empty();
        state.counters[0] = Some(1);
        assert!(check(&[request.clone()], &state, checkpoint).inconsistent.iter().any(|s| s.contains("final INCR")), "unknown absent journal cannot conceal an INCR effect");
        state.logs[0] = request.token.clone();
        state.sets[0] = Some(request.token.clone());
        request.outcome = Outcome::Rejected;
        assert!(check(&[request], &state, checkpoint).inconsistent.iter().any(|s| s.contains("rejected request")));
    }

    #[test]
    fn chaos_checker_rejects_unknown_tokens_and_malformed_histories_or_snapshots() {
        let (events, state, checkpoint) = fixture();
        let mut bad = state.clone();
        bad.logs[1] = "0000000000000099".into();
        assert!(check(&events, &bad, checkpoint).inconsistent.iter().any(|s| s.contains("unknown or wrong-key")));
        let mut bad = state.clone();
        bad.logs[0].push('x');
        assert!(check(&events, &bad, checkpoint).inconsistent.iter().any(|s| s.contains("incomplete journal")));
        let mut bad = state;
        bad.logs[0] = "ééééééééé".into();
        assert!(check(&events, &bad, checkpoint).inconsistent.iter().any(|s| s.contains("malformed journal")));
        for variant in 0..6 {
            let mut bad = events.clone();
            match variant {
                0 => bad[0].key = KEYS,
                1 => bad[0].token.push('x'),
                2 => bad[0].completed = bad[0].invoked - Duration::from_millis(1),
                3 => bad[0].completed = checkpoint + Duration::from_millis(1),
                4 => bad[0].outcome = Outcome::Applied { length: 15, counter: None },
                _ => bad[0].token = bad[1].token.clone(),
            }
            assert!(!check(&bad, &empty(), checkpoint).inconsistent.is_empty(), "bad history variant {variant}");
        }
        let mut bad = empty();
        bad.sets.pop();
        assert!(!check(&[], &bad, checkpoint).inconsistent.is_empty());
    }
}
