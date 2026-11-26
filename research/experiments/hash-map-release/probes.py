"""The probes of a present-field lookup in the two releases' hash maps, modelled for
the keys of redis-benchmark's HSET and HGET tests.

usage: probes.py

The keys are `element:` and a random 12-digit number below 100,000, hashed by
firn's FNV-1a (firn/bytes/bytes.wf, window_hash). The table has the 262,144
buckets that 100,000 pairs need at three quarters of the buckets under both
releases' growth rule, and is probed linearly from the home bucket: the old home
is `hash % buckets`, the new one the top 18 bits of `hash * 11400714819323198485`.
Keys go in in random order, not by the rebuilds of the real map's growth, so the
numbers are of the home function, not a replay of the server.
"""
import random

MASK = (1 << 64) - 1
BUCKETS = 1 << 18
FIBONACCI = 11400714819323198485


def fnv(data):
    hash_value = 14695981039346656037
    for byte in data:
        hash_value = ((hash_value ^ byte) * 1099511628211) & MASK
    return hash_value


def old_home(hash_value):
    return hash_value % BUCKETS


def new_home(hash_value):
    return ((hash_value * FIBONACCI) & MASK) >> (64 - 18)


def probes(hashes, home, seed):
    table = [None] * BUCKETS
    order = list(range(len(hashes)))
    random.Random(seed).shuffle(order)
    for key in order:
        place = home(hashes[key])
        while table[place] is not None:
            place = (place + 1) % BUCKETS
        table[place] = key
    counts = []
    for key, hash_value in enumerate(hashes):
        place = home(hash_value)
        count = 1
        while table[place] != key:
            place = (place + 1) % BUCKETS
            count += 1
        counts.append(count)
    return counts


def selftest():
    assert fnv(b"") == 14695981039346656037
    assert fnv(b"a") == 0xAF63DC4C8601EC8C
    assert 0 <= new_home(MASK) < BUCKETS and 0 <= old_home(MASK) < BUCKETS
    assert probes([5, 5 + BUCKETS, 7], old_home, 0) in ([1, 2, 1], [2, 1, 1])


def main():
    selftest()
    rng = random.Random(1)
    numbers = sorted({rng.randrange(100000) for _ in range(1000000)})
    hashes = [fnv(b"element:%012d" % number) for number in numbers]
    print(f"{len(hashes)} distinct keys in {BUCKETS} buckets")
    for name, home in (("old: hash % buckets", old_home), ("new: top bits of hash * odd", new_home)):
        for seed in (0, 1, 2):
            counts = probes(hashes, home, seed)
            print(f"{name:28} order {seed}: mean {sum(counts) / len(counts):.4f} probes, longest {max(counts)}, "
                  f"{sum(1 for c in counts if c == 1) / len(counts):.3f} at the home bucket")


if __name__ == "__main__":
    main()
