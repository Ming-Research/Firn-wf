//! The sorted-set algebra commands (ZUNIONSTORE, ZINTERSTORE, ZDIFFSTORE,
//! ZUNION, ZINTER, ZDIFF, ZINTERCARD), ZRANGESTORE, ZRANDMEMBER and ZMPOP
//! against Redis 7.2.16 (board item 171).
//!
//! Each case sends the same commands to firn and to the redis-server
//! `REDIS_SERVER` names, which `support::redis_oracle` checks is 7.2.16, and
//! compares every reply byte for byte, under RESP2 and RESP3 and directly, in
//! MULTI/EXEC and from scripts; with the append-only file on it compares the
//! bytes each server appended. The one command whose answer is random,
//! ZRANDMEMBER with a count, is compared by its shape and by what it may hold.

use super::*;
use super::streams::{Server, ask, firn_in, increment, redis, stop, words};

/// A pause the cases use to let a key's expiry pass.
const EXPIRY_WAIT: Duration = Duration::from_millis(120);

/// A small xorshift generator, so a failure names the input that made it.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
        choices[self.below(choices.len())]
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Sends each command to both servers and requires equal replies; `SLEEP`
/// waits for a key's expiry to pass. Returns the replies Redis gave.
fn check(firn: &mut Server, oracle: &mut Server, label: &str, commands: &[Vec<String>]) -> Vec<Vec<u8>> {
    let mut replies = Vec::new();
    for command in commands {
        let arguments: Vec<&str> = command.iter().map(String::as_str).collect();
        if arguments[0] == "SLEEP" {
            std::thread::sleep(EXPIRY_WAIT);
            continue;
        }
        let expected = ask(oracle, &arguments);
        let actual = ask(firn, &arguments);
        if arguments[0] == "HELLO" {
            continue;
        }
        assert_eq!(lossy(&actual), lossy(&expected), "{label}: firn must answer {arguments:?} as Redis 7.2.16 does");
        replies.push(expected);
    }
    replies
}

fn list(texts: &[&str]) -> Vec<Vec<String>> {
    texts.iter().map(|text| words(text)).collect()
}

fn protocol(firn: &mut Server, oracle: &mut Server, version: u8) {
    let hello = format!("HELLO {version}");
    check(firn, oracle, "protocol", &list(&[hello.as_str()]));
}

/// The keys the cases read: sorted sets with shared, disjoint, negative and
/// infinite scores, plain sets, keys of other kinds, a key that vanished with
/// its last member and `out`, a string with an expiry that a store replaces.
fn fixture() -> Vec<Vec<String>> {
    list(&[
        "FLUSHALL",
        "ZADD a 1 x 2 y 3 z 4 w",
        "ZADD b 10 y 20 z 30 v -inf m",
        "ZADD c 1.5 x 2.5 y 3.5 z 0.1 u",
        "ZADD one 7.25 only",
        "ZADD infs +inf p -inf n 1 o",
        "ZADD infs2 -inf p +inf n 2 o",
        "ZADD lex 0 a 0 b 0 c 0 d 0 e",
        "SADD s x y q",
        "SADD s2 y z q",
        "SET str hello",
        "RPUSH list 1 2",
        "ZADD gone 1 x",
        "ZREM gone x",
        "SET out v",
        "PEXPIREAT out 4102444800000",
    ])
}

/// What each case leaves behind to compare, beyond the reply.
fn dump() -> Vec<Vec<String>> {
    list(&[
        "ZRANGE a 0 -1 WITHSCORES", "ZRANGE b 0 -1 WITHSCORES", "ZRANGE c 0 -1 WITHSCORES",
        "ZRANGE out 0 -1 WITHSCORES", "TYPE out", "PEXPIRETIME out", "DBSIZE",
    ])
}

fn case(statement: &str) -> Vec<Vec<String>> {
    let mut commands = fixture();
    commands.push(words(statement));
    commands.extend(dump());
    commands
}

/// What follows the keys of ZUNION, ZINTER and their stores.
const COMBINED: &[&str] = &[
    "2 a b", "3 a b c", "1 a", "2 a nosuch", "2 nosuch a", "2 nosuch nosuch2", "2 a a", "3 a b a", "2 a gone",
    "2 a s", "2 s s2", "3 a s c", "2 one a", "3 c b a", "2 a b WEIGHTS 1 2", "2 a b WEIGHTS 0.5 -1",
    "3 a b c WEIGHTS 1 2 3", "3 a b c WEIGHTS 3 2 1 AGGREGATE MIN", "3 a b c WEIGHTS 3 2 1 AGGREGATE MAX",
    "3 a b c WEIGHTS 3 2 1 AGGREGATE SUM", "2 a a WEIGHTS 2 3", "2 a a WEIGHTS 2 3 AGGREGATE MAX",
    "2 a a AGGREGATE MIN WEIGHTS 2 3", "2 s s2 WEIGHTS 2 3 AGGREGATE MAX", "2 s s2 WEIGHTS 2 3",
    "2 a s WEIGHTS 2 3 AGGREGATE MIN", "2 infs infs2", "2 infs infs2 AGGREGATE MIN", "2 infs infs2 AGGREGATE MAX",
    "1 infs WEIGHTS 0", "2 a b WEIGHTS inf 0", "2 a b WEIGHTS -inf +inf", "2 a b WEIGHTS 1e308 1e308",
    "2 infs infs WEIGHTS 1 -1", "2 a b aggregate max weights 3 4", "2 a b AGGREGATE SUM AGGREGATE MIN",
    "2 a b WEIGHTS 1 2 WEIGHTS 3 4", "2 a b WEIGHTS 1.5 2.5 AGGREGATE MIN", "2 a nosuch WEIGHTS 2 3",
    "0 a", "-1 a", "x a", "1.5 a", "3 a b", "9223372036854775807 a", "2 a b WEIGHTS 1", "2 a b WEIGHTS 1 2 3",
    "2 a b WEIGHTS 1 x", "2 a b WEIGHTS 1 nan", "2 a b WEIGHTS 1 1e400", "2 a b WEIGHTS", "2 a b AGGREGATE",
    "2 a b AGGREGATE AVG", "2 a b FOO", "2 a str", "2 str a", "2 a list", "2 a str WEIGHTS 1 x",
    "2 a nosuch WEIGHTS 1 x", "2 a b LIMIT 1", "2 a b WITHSCORES WEIGHTS 1 2", "2 a b WEIGHTS 1 2 WITHSCORES",
    "2 a b WITHSCORES WITHSCORES",
];

const DIFFERENCES: &[&str] = &[
    "2 a b", "3 a b c", "1 a", "2 a nosuch", "2 nosuch a", "2 a a", "3 a b a", "3 b a a", "2 a s", "2 s a",
    "2 s s2", "2 a gone", "2 one one", "0 a", "x a", "-3 a", "3 a b", "2 a str", "2 str a", "2 a b WEIGHTS 1 2",
    "2 a b AGGREGATE MAX", "2 a b LIMIT 1", "2 a b FOO", "2 a b WITHSCORES WITHSCORES",
];

const CARDINALITIES: &[&str] = &[
    "2 a b", "2 a b LIMIT 1", "2 a b LIMIT 0", "2 a b LIMIT 100", "2 a b LIMIT -1", "2 a b LIMIT x", "2 a b limit 2",
    "3 a b c", "3 a b c LIMIT 2", "1 a", "1 a LIMIT 3", "1 nosuch", "2 a nosuch", "2 a s", "2 s s2", "2 a a", "0 a",
    "x a", "3 a b", "2 a b LIMIT", "2 a b FOO", "2 a b WITHSCORES", "2 a b WEIGHTS 1 2", "2 a b AGGREGATE MAX",
    "2 a str", "2 a b LIMIT 1 LIMIT 2", "2 a b LIMIT 9223372036854775807", "2 infs infs2",
];

fn algebra_cases() -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for (name, stored) in [("ZUNION", "ZUNIONSTORE"), ("ZINTER", "ZINTERSTORE")] {
        for tail in COMBINED {
            commands.extend(case(&format!("{stored} out {tail}")));
            commands.extend(case(&format!("{name} {tail}")));
            commands.extend(case(&format!("{name} {tail} WITHSCORES")));
        }
    }
    for tail in DIFFERENCES {
        commands.extend(case(&format!("ZDIFFSTORE out {tail}")));
        commands.extend(case(&format!("ZDIFF {tail}")));
        commands.extend(case(&format!("ZDIFF {tail} WITHSCORES")));
    }
    for tail in CARDINALITIES {
        commands.extend(case(&format!("ZINTERCARD {tail}")));
    }
    // A store whose destination is also a source reads it first.
    for statement in [
        "ZUNIONSTORE a 2 a b", "ZINTERSTORE a 2 a b WEIGHTS 2 3", "ZDIFFSTORE a 2 a b", "ZDIFFSTORE b 2 a b",
        "ZUNIONSTORE s 2 a s", "ZINTERSTORE a 2 a nosuch", "ZUNIONSTORE str 2 a b", "ZINTERSTORE list 2 a b",
        "ZDIFFSTORE s 1 a",
    ] {
        commands.extend(case(statement));
    }
    // A destination that is absent stays absent when the result is empty.
    for statement in [
        "ZUNIONSTORE fresh 1 nosuch", "ZINTERSTORE fresh 2 a nosuch", "ZDIFFSTORE fresh 2 nosuch a",
        "ZINTERSTORE fresh 2 a lex", "ZRANGESTORE fresh a 9 10",
    ] {
        commands.extend(case(statement));
        commands.extend(list(&["EXISTS fresh", "DBSIZE"]));
    }
    commands
}

fn range_store_cases() -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for statement in [
        "ZRANGESTORE out a 0 -1", "ZRANGESTORE out a 1 2", "ZRANGESTORE out a 0 -1 REV", "ZRANGESTORE out a 5 10",
        "ZRANGESTORE out a -2 -1", "ZRANGESTORE out a 2 1", "ZRANGESTORE out a 0 0 REV",
        "ZRANGESTORE out a 2 3 BYSCORE", "ZRANGESTORE out a (1 +inf BYSCORE", "ZRANGESTORE out a -inf (3 BYSCORE",
        "ZRANGESTORE out a 3 1 BYSCORE REV", "ZRANGESTORE out a 3 1 BYSCORE", "ZRANGESTORE out a 1 4 BYSCORE LIMIT 1 2",
        "ZRANGESTORE out a 1 4 BYSCORE LIMIT 1 -1", "ZRANGESTORE out a 1 4 BYSCORE LIMIT 9 2",
        "ZRANGESTORE out lex [b (e BYLEX", "ZRANGESTORE out lex + - BYLEX REV", "ZRANGESTORE out lex - + BYLEX LIMIT 1 2",
        "ZRANGESTORE out lex - + BYLEX LIMIT 0 -1", "ZRANGESTORE out lex b e BYLEX", "ZRANGESTORE out lex x y",
        "ZRANGESTORE out a 0 -1 LIMIT 0 1", "ZRANGESTORE out a 0 -1 WITHSCORES", "ZRANGESTORE out a 1 4 BYSCORE LIMIT 1 x",
        "ZRANGESTORE out a 1 4 BYSCORE LIMIT 1", "ZRANGESTORE out a x 1", "ZRANGESTORE out a 0 y",
        "ZRANGESTORE out a 0 -1 BYSCORE BYLEX", "ZRANGESTORE out a 0 -1 REV REV", "ZRANGESTORE out a 0 -1 FOO",
        "ZRANGESTORE out a 1 x BYSCORE", "ZRANGESTORE out s 0 -1", "ZRANGESTORE out str 0 -1",
        "ZRANGESTORE out nosuch 0 -1", "ZRANGESTORE out gone 0 -1", "ZRANGESTORE a a 0 1", "ZRANGESTORE b a 0 -1",
        "ZRANGESTORE s a 0 -1", "ZRANGESTORE out a 0", "ZRANGESTORE out a", "ZRANGESTORE out",
        "zrangestore out a 0 -1 rev", "ZRANGESTORE out infs -inf +inf BYSCORE", "ZRANGESTORE out b -inf +inf BYSCORE REV",
    ] {
        commands.extend(case(statement));
    }
    commands
}

fn pop_cases() -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for statement in [
        "ZMPOP 1 a MIN", "ZMPOP 1 a MAX", "ZMPOP 1 a MIN COUNT 2", "ZMPOP 1 a MAX COUNT 3", "ZMPOP 1 a MIN COUNT 100",
        "ZMPOP 2 nosuch a MAX COUNT 2", "ZMPOP 3 nosuch gone b MIN", "ZMPOP 2 nosuch nosuch2 MIN",
        "ZMPOP 2 a b MIN COUNT 10", "ZMPOP 2 b a MAX", "ZMPOP 2 str a MIN", "ZMPOP 2 a str MIN", "ZMPOP 2 nosuch str MIN",
        "ZMPOP 1 str MIN", "ZMPOP 1 s MAX", "ZMPOP 1 list MIN", "ZMPOP 2 a a MIN COUNT 10", "ZMPOP 1 one MIN",
        "ZMPOP 1 one MAX COUNT 5", "zmpop 1 a min count 2", "ZMPOP 1 a Max Count 1", "ZMPOP 0 a MIN", "ZMPOP -1 a MIN",
        "ZMPOP x a MIN", "ZMPOP 1.5 a MIN", "ZMPOP 5 a MIN", "ZMPOP 2 a MIN", "ZMPOP 1 a", "ZMPOP 1 a FOO",
        "ZMPOP 1 a MIN COUNT 0", "ZMPOP 1 a MIN COUNT -1", "ZMPOP 1 a MIN COUNT x", "ZMPOP 1 a MIN COUNT",
        "ZMPOP 1 a MIN COUNT 1 COUNT 2", "ZMPOP 1 a MIN FOO 1", "ZMPOP 1 a MIN COUNT 9223372036854775807",
        "ZMPOP 1 a MIN COUNT 9223372036854775808", "ZMPOP 9223372036854775807 a MIN", "ZMPOP 1 infs MIN COUNT 3",
        "ZMPOP 1 infs MAX COUNT 3", "ZMPOP 1 a", "ZMPOP 1", "ZMPOP",
    ] {
        commands.extend(case(statement));
    }
    commands
}

fn expiry_cases() -> Vec<Vec<String>> {
    list(&[
        "FLUSHALL", "ZADD e1 1 a 2 b", "ZADD e2 5 b 6 c", "ZADD d 1 q", "PEXPIRE e1 30", "PEXPIRE d 30", "SLEEP",
        "ZUNION 2 e1 e2 WITHSCORES", "EXISTS e1",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "SLEEP", "ZINTERCARD 2 e1 e2", "EXISTS e1",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "SLEEP", "ZUNIONSTORE d 2 e1 e2", "ZRANGE d 0 -1 WITHSCORES", "TTL d",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "ZADD d 1 q", "PEXPIRE d 30", "SLEEP", "ZINTERSTORE d 2 e1 e2", "EXISTS d e1",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "ZADD d 1 q", "PEXPIRE d 30", "SLEEP", "ZDIFFSTORE d 2 e1 e2", "EXISTS d e1",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "SLEEP", "ZRANGESTORE d e1 0 -1", "EXISTS d e1",
        "ZADD e1 1 a 2 b", "PEXPIRE e1 30", "SLEEP", "ZRANDMEMBER e1", "ZRANDMEMBER e1 3", "EXISTS e1",
        // A key whose expiry has passed is skipped by ZMPOP, and removed.
        "ZADD e1 1 a", "ZADD e2 2 b", "PEXPIRE e1 30", "SLEEP", "ZMPOP 2 e1 e2 MIN", "EXISTS e1 e2",
        "ZADD e1 1 a", "PEXPIRE e1 30", "SLEEP", "ZMPOP 1 e1 MIN", "EXISTS e1",
    ])
}

fn transaction_cases() -> Vec<Vec<String>> {
    let mut commands = fixture();
    commands.extend(list(&[
        "MULTI", "ZUNIONSTORE out 2 a b WEIGHTS 1 2", "ZINTERSTORE out2 2 a b", "ZDIFFSTORE out3 2 a b",
        "ZUNION 2 a b WITHSCORES", "ZINTER 2 a b WITHSCORES", "ZDIFF 2 a b WITHSCORES", "ZINTERCARD 2 a b LIMIT 1",
        "ZRANGESTORE out4 a 0 -1", "ZRANDMEMBER one", "ZRANDMEMBER one -2 WITHSCORES", "ZMPOP 1 a MIN", "ZMPOP 2 nosuch b MAX COUNT 2",
        "ZUNION 0 a", "ZUNIONSTORE out 2 a str", "ZMPOP 1 a MIN COUNT 0", "ZRANGESTORE out5 a 0 -1 WITHSCORES", "EXEC",
        "ZRANGE out 0 -1 WITHSCORES", "ZRANGE out2 0 -1 WITHSCORES", "ZRANGE out3 0 -1 WITHSCORES",
        "ZRANGE out4 0 -1 WITHSCORES", "ZRANGE a 0 -1 WITHSCORES", "EXISTS out5",
        // A queue-time refusal aborts the whole transaction.
        "MULTI", "ZUNIONSTORE out", "ZUNIONSTORE out6 1 a", "EXEC", "EXISTS out6",
        "MULTI", "ZMPOP 1", "ZMPOP 1 b MIN", "EXEC", "ZRANGE b 0 -1",
        "MULTI", "ZRANDMEMBER", "EXEC",
        "MULTI", "ZINTERCARD", "EXEC",
        "MULTI", "ZRANGESTORE out a", "EXEC",
        "MULTI", "ZDIFF 1 a", "ZINTERCARD 1 a", "EXEC",
    ]));
    commands
}

/// The numkeys of a script's call are its own arguments, so a script names the
/// keys it reads either way; undeclared keys are allowed, as in Redis.
fn script_cases() -> Vec<Vec<String>> {
    let mut commands = fixture();
    for call in [
        "return redis.call('ZUNIONSTORE', KEYS[1], 2, KEYS[2], KEYS[3], 'WEIGHTS', 1, 2)",
        "return redis.call('ZINTERSTORE', KEYS[1], 2, KEYS[2], KEYS[3], 'AGGREGATE', 'MAX')",
        "return redis.call('ZDIFFSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.call('ZUNION', 2, KEYS[2], KEYS[3], 'WITHSCORES')",
        "return redis.call('ZINTER', 2, KEYS[2], KEYS[3], 'WEIGHTS', 2, 3, 'WITHSCORES')",
        "return redis.call('ZDIFF', 2, KEYS[2], KEYS[3], 'WITHSCORES')",
        "return redis.call('ZINTERCARD', 2, KEYS[2], KEYS[3], 'LIMIT', 1)",
        "return redis.call('ZRANGESTORE', KEYS[1], KEYS[2], 0, -1)",
        "return redis.call('ZRANGESTORE', KEYS[1], KEYS[2], '-inf', '+inf', 'BYSCORE', 'REV', 'LIMIT', 0, 2)",
        "return redis.call('ZMPOP', 2, KEYS[3], KEYS[2], 'MAX', 'COUNT', 2)",
        "return redis.call('ZMPOP', 1, KEYS[2], 'MIN')",
        "return redis.call('ZRANDMEMBER', 'one')",
        "return redis.call('ZRANDMEMBER', 'one', -2, 'WITHSCORES')",
        "return redis.call('ZRANDMEMBER', 'nosuch', 2)",
        "return redis.pcall('ZUNIONSTORE', KEYS[1], 0, KEYS[2])",
        "return redis.pcall('ZUNIONSTORE', KEYS[1], 2, KEYS[2], 'str')",
        "return redis.pcall('ZINTERSTORE', KEYS[1], 2, KEYS[2], KEYS[3], 'WEIGHTS', 1, 'x')",
        "return redis.pcall('ZDIFF', 2, KEYS[2], KEYS[3], 'WEIGHTS', 1, 2)",
        "return redis.pcall('ZMPOP', 1, KEYS[2], 'MIDDLE')",
        "return redis.pcall('ZMPOP', 0, KEYS[2], 'MIN')",
        "return redis.pcall('ZRANGESTORE', KEYS[1], KEYS[2], 0, -1, 'WITHSCORES')",
        "return redis.pcall('ZRANDMEMBER', 'str')",
        "return redis.pcall('ZUNION', 2, KEYS[2])",
        "return redis.pcall('ZINTERCARD', 2, KEYS[2], KEYS[3], 'LIMIT', -1)",
        "local n = redis.call('ZUNIONSTORE', KEYS[1], 2, KEYS[2], KEYS[3]); return {n, redis.call('ZRANGE', KEYS[1], 0, -1, 'WITHSCORES')}",
        "return redis.call('ZUNION', 2, 'a', 'b', 'WITHSCORES')",
    ] {
        commands.push(vec!["EVAL".into(), call.into(), "3".into(), "out".into(), "a".into(), "b".into()]);
        commands.extend(dump());
    }
    commands
}

/// A stream of random commands over keys of several sizes, kinds and scores.
/// The equal-sized `eq` keys make a float sum of three or more inputs depend
/// on the order the sets are visited.
fn generated(seed: u64, count: usize) -> Vec<Vec<String>> {
    let mut rng = Rng(seed);
    let mut commands = list(&["FLUSHALL"]);
    let scores = [
        "0", "1", "2", "-1", "-7", "0.5", "0.1", "0.2", "0.3", "0.7", "3.14159", "-2.5", "1e15", "1e-9", "123456.789",
        "0.30000000000000004", "1.7976931348623157e308", "-1.7976931348623157e308", "inf", "-inf",
    ];
    for key in 0..5 {
        let size = 10 + rng.below(40);
        for _ in 0..size {
            let member = format!("m{}", rng.below(70));
            let score = if rng.below(25) == 0 { rng.pick(&["inf", "-inf"]) } else { rng.pick(&scores[..18]) };
            commands.push(vec!["ZADD".into(), format!("g{key}"), score.into(), member]);
        }
    }
    for key in 0..4 {
        for member in 0..12 {
            let tenths = rng.below(2000) as i64 - 1000;
            commands.push(vec!["ZADD".into(), format!("eq{key}"), format!("{}", tenths as f64 / 10.0), format!("m{}", member + key)]);
        }
    }
    for key in 0..2 {
        for _ in 0..20 {
            commands.push(vec!["SADD".into(), format!("ps{key}"), format!("m{}", rng.below(60))]);
        }
    }
    let keys = ["g0", "g1", "g2", "g3", "g4", "eq0", "eq1", "eq2", "eq3", "ps0", "ps1", "none", "g0", "eq0"];
    let weights = ["1", "2", "0.5", "-1", "0", "3.25", "inf", "-inf", "1e308", "0.1", "0.3", "7"];
    let operations = ["ZUNIONSTORE", "ZINTERSTORE", "ZDIFFSTORE", "ZUNION", "ZINTER", "ZDIFF", "ZINTERCARD"];
    for step in 0..count {
        let operation = rng.pick(&operations);
        let stored = operation.ends_with("STORE");
        let numkeys = 1 + rng.below(5);
        let mut command = vec![operation.to_owned()];
        let destination = format!("d{}", step % 3);
        if stored {
            command.push(destination.clone());
        }
        command.push(numkeys.to_string());
        for _ in 0..numkeys {
            command.push(rng.pick(&keys).to_owned());
        }
        let combining = operation.starts_with("ZUNION") || operation.starts_with("ZINTER") && operation != "ZINTERCARD";
        if combining && rng.below(2) == 0 {
            command.push("WEIGHTS".into());
            for _ in 0..numkeys {
                command.push(rng.pick(&weights).to_owned());
            }
        }
        if combining && rng.below(2) == 0 {
            command.push("AGGREGATE".into());
            command.push(rng.pick(&["SUM", "MIN", "MAX"]).to_owned());
        }
        if !stored && operation != "ZINTERCARD" && rng.below(3) != 0 {
            command.push("WITHSCORES".into());
        }
        if operation == "ZINTERCARD" && rng.below(2) == 0 {
            command.push("LIMIT".into());
            command.push(rng.below(6).to_string());
        }
        commands.push(command);
        if stored {
            commands.push(vec!["ZRANGE".into(), destination.clone(), "0".into(), "-1".into(), "WITHSCORES".into()]);
        }
        if step % 17 == 0 {
            let source = rng.pick(&["g0", "g1", "g2", "eq0"]).to_owned();
            commands.push(words(&format!("ZRANGESTORE r{} {source} {} {}", step % 2, rng.below(5), rng.below(5))));
            commands.push(words(&format!("ZRANGE r{} 0 -1 WITHSCORES", step % 2)));
        }
        if step % 23 == 0 {
            let keys = format!("{} {}", rng.pick(&["g1", "g2", "eq1", "none"]), rng.pick(&["g3", "eq2", "g4"]));
            let count = 1 + rng.below(6);
            commands.push(words(&format!("ZMPOP 2 {keys} {} COUNT {count}", rng.pick(&["MIN", "MAX"]))));
        }
    }
    commands
}

#[test]
fn zset_algebra_answers_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), false);
    let mut server = firn_in(&program, false);
    let mut replies = 0;
    for version in [2_u8, 3] {
        protocol(&mut server, &mut oracle, version);
        let label = format!("RESP{version}");
        replies += check(&mut server, &mut oracle, &label, &algebra_cases()).len();
        replies += check(&mut server, &mut oracle, &label, &range_store_cases()).len();
        replies += check(&mut server, &mut oracle, &label, &pop_cases()).len();
        replies += check(&mut server, &mut oracle, &label, &expiry_cases()).len();
        replies += check(&mut server, &mut oracle, &label, &transaction_cases()).len();
        replies += check(&mut server, &mut oracle, &label, &script_cases()).len();
    }
    // The cases separate the commands from an unknown-command refusal.
    assert!(replies > 3000, "{replies} replies compared");
    let mut hit = check(&mut server, &mut oracle, "sanity", &list(&["FLUSHALL", "ZADD a 1 x", "ZUNION 1 a WITHSCORES"]));
    assert!(!lossy(&hit.pop().unwrap()).starts_with('-'), "the oracle answers the union");
    stop(server);
    stop(oracle);
}

#[test]
fn zset_algebra_generated_inputs_answer_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), false);
    let mut server = firn_in(&program, false);
    for (version, seed) in [(2_u8, 0x2545_f491_4f6c_dd1d_u64), (3, 0x9e37_79b9_7f4a_7c15)] {
        protocol(&mut server, &mut oracle, version);
        let label = format!("RESP{version} seed {seed:#x}");
        check(&mut server, &mut oracle, &label, &generated(seed, 400));
    }
    stop(server);
    stop(oracle);
}

/// The append-only file records the store commands and ZRANGESTORE as
/// received, ZMPOP as the ZPOPMIN or ZPOPMAX it came to with the number
/// popped, and nothing for a command that changed nothing; a restart replays
/// the file to the same keys. Expired keys are left out: a sorted set can only
/// be given a short expiry, and Redis's and firn's own expiry passes may remove
/// it before the command, which changes what is appended.
#[test]
fn zset_algebra_records_its_writes_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), true);
    let mut server = firn_in(&program, true);
    let mut commands = algebra_cases();
    commands.extend(range_store_cases());
    commands.extend(pop_cases());
    commands.extend(transaction_cases());
    commands.extend(script_cases());
    commands.extend(generated(0x1234_5678_9abc_def1, 150));
    commands.extend(list(&[
        "FLUSHALL", "ZADD k1 1 a 2 b 3 c", "ZADD k2 5 b 6 c", "ZMPOP 2 nosuch k1 MIN COUNT 2", "ZMPOP 1 k1 MAX COUNT 9",
        "ZMPOP 1 k1 MIN", "ZMPOP 2 k2 k1 MAX", "ZUNIONSTORE nothing 1 absent", "ZRANGESTORE nothing k2 5 6",
        "ZINTERSTORE k2 2 k2 absent", "EXISTS k2 nothing",
    ]));
    let mut keys = Vec::new();
    for key in ["a", "b", "c", "out", "d0", "d1", "d2", "r0", "r1", "k1", "k2", "g0", "g1", "g2", "eq0", "eq1", "fresh"] {
        keys.push(words(&format!("ZRANGE {key} 0 -1 WITHSCORES")));
    }
    check(&mut server, &mut oracle, "AOF", &commands);
    let before = check(&mut server, &mut oracle, "keys", &keys);
    stop(server);
    stop(oracle);
    let expected = increment(directory.path());
    let actual = increment(program.working_directory());
    assert_eq!(aof_records_without_select_zero(&actual), aof_records_without_select_zero(&expected),
        "firn must append the sorted-set commands Redis 7.2.16 appends");
    let mut restarted = firn_in(&program, true);
    for (key, want) in keys.iter().zip(&before) {
        let arguments: Vec<&str> = key.iter().map(String::as_str).collect();
        assert_eq!(lossy(&ask(&mut restarted, &arguments)), lossy(want), "after a restart {arguments:?}");
    }
    stop(restarted);
}

/// A reply with the text of its strings and numbers set aside, to compare the
/// framing of two replies whose contents are random.
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    Null,
    Integer,
    Bulk(String),
    Double(String),
    Line(String),
    Array(Vec<Reply>),
}

fn parse_at(bytes: &[u8], at: &mut usize) -> Reply {
    let end = bytes[*at..].windows(2).position(|pair| pair == b"\r\n").expect("a line ends") + *at;
    let head = String::from_utf8_lossy(&bytes[*at..end]).into_owned();
    *at = end + 2;
    let rest = &head[1..];
    match head.as_bytes()[0] {
        b'+' | b'-' => Reply::Line(head),
        b':' => Reply::Integer,
        b'_' => Reply::Null,
        b',' => Reply::Double(rest.to_owned()),
        b'$' => {
            let length: i64 = rest.parse().expect("a bulk length");
            if length < 0 {
                return Reply::Null;
            }
            let text = String::from_utf8_lossy(&bytes[*at..*at + length as usize]).into_owned();
            *at += length as usize + 2;
            Reply::Bulk(text)
        }
        b'*' | b'~' | b'>' => {
            let length: i64 = rest.parse().expect("an array length");
            if length < 0 {
                return Reply::Null;
            }
            Reply::Array((0..length).map(|_| parse_at(bytes, at)).collect())
        }
        other => panic!("unexpected reply type {:?} in {:?}", other as char, lossy(bytes)),
    }
}

fn parse(bytes: &[u8]) -> Reply {
    let mut at = 0;
    let reply = parse_at(bytes, &mut at);
    assert_eq!(at, bytes.len(), "one whole reply: {:?}", lossy(bytes));
    reply
}

fn skeleton(reply: &Reply) -> Reply {
    match reply {
        Reply::Bulk(_) => Reply::Bulk(String::new()),
        Reply::Double(_) => Reply::Double(String::new()),
        Reply::Array(items) => Reply::Array(items.iter().map(skeleton).collect()),
        other => other.clone(),
    }
}

/// The members, and their score texts when scored, a ZRANDMEMBER reply holds,
/// in the order given: a pair is a nested array under RESP3 and two elements of
/// one array under RESP2.
fn drawn(reply: &Reply, scored: bool) -> Vec<(String, Option<String>)> {
    match reply {
        Reply::Bulk(member) => vec![(member.clone(), None)],
        Reply::Array(items) if !scored => items.iter().map(|item| match item {
            Reply::Bulk(member) => (member.clone(), None),
            other => panic!("a member, not {other:?}"),
        }).collect(),
        Reply::Array(items) if matches!(items.first(), Some(Reply::Array(_))) => items.iter().map(|item| match item {
            Reply::Array(pair) => match pair.as_slice() {
                [Reply::Bulk(member), Reply::Double(score) | Reply::Bulk(score)] => (member.clone(), Some(score.clone())),
                other => panic!("a member and its score, not {other:?}"),
            },
            other => panic!("a pair, not {other:?}"),
        }).collect(),
        Reply::Array(items) => items.chunks(2).map(|pair| match pair {
            [Reply::Bulk(member), Reply::Bulk(score)] => (member.clone(), Some(score.clone())),
            other => panic!("a member and its score, not {other:?}"),
        }).collect(),
        other => panic!("not a draw: {other:?}"),
    }
}

fn score_text(reply: &[u8]) -> String {
    match parse(reply) {
        Reply::Bulk(text) | Reply::Double(text) => text,
        other => panic!("a score, not {other:?}"),
    }
}

/// ZRANDMEMBER's draws are random, so each is held to what Redis's own reply
/// fixes: the framing, the number of members, that they are members with their
/// scores, and that a positive count draws none twice.
#[test]
fn zrandmember_draws_have_redis_shape() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), false);
    let mut server = firn_in(&program, false);
    let sizes = [("small", 30_usize), ("large", 500)];
    for version in [2_u8, 3] {
        protocol(&mut server, &mut oracle, version);
        let mut commands = list(&["FLUSHALL", "ZADD one 2.5 only", "SET str v", "SADD plain a b"]);
        for (key, size) in sizes {
            for index in 0..size {
                commands.push(words(&format!("ZADD {key} {} m{index}", index as f64 * 1.5)));
            }
        }
        check(&mut server, &mut oracle, "fill", &commands);
        // What has one answer is compared exactly.
        check(&mut server, &mut oracle, "exact", &list(&[
            "ZRANDMEMBER nosuch", "ZRANDMEMBER nosuch 3", "ZRANDMEMBER nosuch -3 WITHSCORES", "ZRANDMEMBER one",
            "ZRANDMEMBER one 0", "ZRANDMEMBER one 0 WITHSCORES", "ZRANDMEMBER one 5", "ZRANDMEMBER one 5 WITHSCORES",
            "ZRANDMEMBER one -3", "ZRANDMEMBER one -3 WITHSCORES", "ZRANDMEMBER one 9223372036854775807",
            "ZRANDMEMBER str", "ZRANDMEMBER str 2", "ZRANDMEMBER plain", "ZRANDMEMBER small x", "ZRANDMEMBER small WITHSCORES",
            "ZRANDMEMBER small 1 WITHSCORE", "ZRANDMEMBER small 1 WITHSCORES extra", "ZRANDMEMBER small 1 2",
            "ZRANDMEMBER small -9223372036854775808", "ZRANDMEMBER small 9223372036854775808",
            "ZRANDMEMBER small 4611686018427387904 WITHSCORES", "ZRANDMEMBER small -4611686018427387904 WITHSCORES",
            "ZRANDMEMBER", "ZRANDMEMBER one 1 withscores", "ZRANDMEMBER small 1 WITHSCORES WITHSCORES",
        ]));
        for (key, size) in sizes {
            let mut expected = std::collections::HashMap::new();
            for index in 0..size {
                let reply = ask(&mut oracle, &["ZSCORE", key, &format!("m{index}")]);
                expected.insert(format!("m{index}"), score_text(&reply));
            }
            let counts: Vec<i64> = if size == 30 {
                vec![1, 2, 5, 10, 11, 15, 29, 30, 31, 1000, -1, -5, -30, -31, -100]
            } else {
                vec![1, 3, 100, 166, 167, 250, 499, 500, 501, -3, -499, -600]
            };
            for count in counts {
                for scores in [false, true] {
                    for _ in 0..8 {
                        let text = count.to_string();
                        let mut command = vec!["ZRANDMEMBER", key, text.as_str()];
                        if scores {
                            command.push("WITHSCORES");
                        }
                        let want = parse(&ask(&mut oracle, &command));
                        let got_bytes = ask(&mut server, &command);
                        let got = parse(&got_bytes);
                        let label = format!("RESP{version} {command:?}: {}", lossy(&got_bytes));
                        assert_eq!(skeleton(&got), skeleton(&want), "{label}");
                        let members = drawn(&got, scores);
                        let wanted = if count >= 0 { (count as usize).min(size) } else { count.unsigned_abs() as usize };
                        assert_eq!(members.len(), wanted, "{label}");
                        for (member, score) in &members {
                            let held = expected.get(member).unwrap_or_else(|| panic!("{member} is no member: {label}"));
                            if let Some(score) = score {
                                assert_eq!(score, held, "{member}'s score: {label}");
                            }
                        }
                        if count >= 0 {
                            let mut seen: Vec<&String> = members.iter().map(|(member, _)| member).collect();
                            seen.sort();
                            seen.dedup();
                            assert_eq!(seen.len(), members.len(), "a positive count draws none twice: {label}");
                        }
                    }
                }
            }
            // One draw at a time is not stuck on one member.
            let mut seen = std::collections::HashSet::new();
            for _ in 0..80 {
                if let Reply::Bulk(member) = parse(&ask(&mut server, &["ZRANDMEMBER", key])) {
                    seen.insert(member);
                }
            }
            assert!(seen.len() >= 8, "{key}: 80 draws gave {} members", seen.len());
        }
        // The same framing inside a transaction and a script.
        for command in [
            list(&["MULTI", "ZRANDMEMBER small 3", "ZRANDMEMBER small -2 WITHSCORES", "ZRANDMEMBER small", "EXEC"]),
            list(&["EVAL return{redis.call('ZRANDMEMBER',KEYS[1],3),redis.call('ZRANDMEMBER',KEYS[1])} 1 small"]),
            list(&["EVAL return(redis.call('ZRANDMEMBER',KEYS[1],-4,'WITHSCORES')) 1 small"]),
        ] {
            for step in command {
                let arguments: Vec<&str> = step.iter().map(String::as_str).collect();
                let want = ask(&mut oracle, &arguments);
                let got = ask(&mut server, &arguments);
                if arguments[0] == "EXEC" || arguments[0] == "EVAL" {
                    assert_eq!(skeleton(&parse(&got)), skeleton(&parse(&want)), "RESP{version} {arguments:?}");
                } else {
                    assert_eq!(lossy(&got), lossy(&want), "RESP{version} {arguments:?}");
                }
            }
        }
    }
    stop(server);
    stop(oracle);
}

/// DENYOOM and WRITE flags: the four stores are refused over the memory limit,
/// the reads and ZMPOP are not, a script's store is refused as Redis refuses
/// it, and EVAL_RO refuses the five writes while it runs the reads.
#[test]
fn zset_commands_follow_redis_flags_under_a_memory_limit_and_read_only_scripts() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), false);
    let mut server = firn_in(&program, false);
    let mut commands = fixture();
    commands.extend(list(&["CONFIG SET maxmemory 1"]));
    for statement in [
        "ZUNIONSTORE out 2 a b", "ZINTERSTORE out 2 a b", "ZDIFFSTORE out 2 a b", "ZRANGESTORE out a 0 -1",
        "ZUNIONSTORE out 0 a", "ZMPOP 1 a MIN", "ZMPOP 2 nosuch gone MAX", "ZUNION 2 a b WITHSCORES", "ZINTER 2 a b",
        "ZDIFF 2 a b", "ZINTERCARD 2 a b", "ZRANDMEMBER one", "ZRANDMEMBER one 3",
    ] {
        commands.push(words(statement));
        commands.push(words("EXISTS out"));
    }
    // Inside a transaction every queued command is refused over the limit.
    commands.extend(list(&["MULTI", "ZUNIONSTORE out 2 a b", "EXEC", "MULTI", "ZUNION 2 a b", "EXEC"]));
    for script in [
        "return redis.pcall('ZUNIONSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZINTERSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZDIFFSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZRANGESTORE', KEYS[1], KEYS[2], 0, -1)",
        "return redis.pcall('ZMPOP', 1, KEYS[2], 'MIN')",
        "return redis.pcall('ZUNION', 2, KEYS[2], KEYS[3])",
    ] {
        commands.push(vec!["EVAL".into(), script.into(), "3".into(), "out".into(), "a".into(), "b".into()]);
    }
    commands.extend(list(&["CONFIG SET maxmemory 0"]));
    for script in [
        "return redis.pcall('ZUNIONSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZINTERSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZDIFFSTORE', KEYS[1], 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZRANGESTORE', KEYS[1], KEYS[2], 0, -1)",
        "return redis.pcall('ZMPOP', 1, KEYS[2], 'MIN')",
        "return redis.pcall('ZUNION', 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZINTER', 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZDIFF', 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZINTERCARD', 2, KEYS[2], KEYS[3])",
        "return redis.pcall('ZRANDMEMBER', 'one')",
    ] {
        for command in ["EVAL_RO", "EVAL"] {
            commands.push(vec![command.into(), script.into(), "3".into(), "out".into(), "a".into(), "b".into()]);
        }
    }
    // A draw from a key the script declares; `one` has a single member, so the
    // answer is the same on both servers.
    for command in ["EVAL_RO", "EVAL"] {
        commands.push(vec![command.into(), "return redis.pcall('ZRANDMEMBER', KEYS[1], 3, 'WITHSCORES')".into(), "1".into(), "one".into()]);
    }
    commands.extend(dump());
    check(&mut server, &mut oracle, "flags", &commands);
    stop(server);
    stop(oracle);
}
