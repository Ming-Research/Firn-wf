//! Pure APPEND serial witness shared by durability and chaos (firn-safety-showcase).

use std::collections::HashSet;
use std::time::Instant;

pub(crate) const SHARED_KEYS: usize = 4;
pub(crate) const TOKEN_BYTES: usize = 16;

#[derive(Clone)]
pub(crate) struct Append {
    pub(crate) key: usize,
    pub(crate) token: String,
    pub(crate) length: i64,
    pub(crate) invoked: Instant,
    pub(crate) completed: Instant,
}

/// A serial witness independent of firn: a key's reply ranks must be exactly
/// 1..N and its bytes must be the corresponding tokens in that order. The
/// rank order must also fit the invocation/completion intervals. In rank
/// order, the latest invocation in each prefix must be no later than that
/// operation's completion; otherwise a higher rank finished before a lower
/// rank was invoked. This gives a linearization within each key's intervals;
/// independent keys compose, without searching possible global orders.
pub(crate) fn check_appends(history: &[Append], final_values: &[String]) -> Result<(), String> {
    if final_values.len() != SHARED_KEYS {
        return Err("wrong number of final keys".to_owned());
    }
    let mut tokens = HashSet::new();
    for event in history {
        if event.key >= SHARED_KEYS || event.token.len() != TOKEN_BYTES
            || event.completed < event.invoked || !tokens.insert(&event.token) {
            return Err("invalid or duplicate history token/interval/key".to_owned());
        }
    }
    for (key, final_value) in final_values.iter().enumerate() {
        let mut events: Vec<_> = history.iter().filter(|event| event.key == key).collect();
        events.sort_unstable_by_key(|event| event.length);
        let mut expected = String::new();
        let mut latest_invocation = None;
        for (index, event) in events.iter().enumerate() {
            let expected_length = ((index + 1) * TOKEN_BYTES) as i64;
            if event.length != expected_length {
                return Err(format!("key {key}: duplicate/missing reply rank: wanted \
                    {expected_length}, got {}", event.length));
            }
            let latest = latest_invocation.map_or(event.invoked, |previous: Instant| {
                previous.max(event.invoked)
            });
            if latest > event.completed {
                return Err(format!("key {key}: reply order contradicts real-time order"));
            }
            latest_invocation = Some(latest);
            expected.push_str(&event.token);
        }
        if *final_value != expected {
            return Err(format!("key {key}: final value differs from acknowledged token order \
                (expected {} bytes, got {})", expected.len(), final_value.len()));
        }
    }
    Ok(())
}

