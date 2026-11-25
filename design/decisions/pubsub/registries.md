# Pub/sub registries

Where firn keeps channel, pattern and shard-channel subscriptions, and the identities of subscribed connections.

### decision pubsub-registries-are-growing-hash-maps
Subscribed-connection records and channel registries are growing standard hash maps, with u64 registration ids that are allocated in order and never reused within a process.
- rejected: an agent proposed the standard library's reusable Slab; the owner rejected it, because at the pinned Whitefoot a Slab cannot grow past its first capacity, which does not fit a growing dictionary.
- rejected: an agent proposed a new fixed ceiling on subscribers; the owner rejected it, because the existing client limit bounds accepted connections, not live registrations, so it gives no such ceiling.

### decision pubsub-channels-patterns-shards-kept-apart
Ordinary channels, patterns and shard channels each have their own server-wide index and a reverse list per connection, and a message is delivered once for each pattern it matches.
- rejected: an agent proposed one namespace for ordinary and shard channels; the owner rejected it, because a shard publication must stay apart from ordinary subscribers.
- rejected: an agent proposed counting only distinct recipient connections; the owner rejected it, because a pattern fanout must count each matching path, and a duplicate subscription must only be acknowledged, without adding a path.