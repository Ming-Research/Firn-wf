Decision: A script's commands read the clock as it was when the script began, as Redis 7.0.15 freezes time for a script, because every command of one script must judge expiry and relative deadlines against one instant.

Decision: A key a script's command finds expired is removed at once and its `DEL` appended to the script's effects, as Redis 7.0.15 removes a key a script's lookup finds expired, because the effects must replay to the state the script left.

Decision: The records a script appends are wrapped in `MULTI` and `EXEC` by the script's caller when it appended more than one, as Redis 7.0.15 propagates a script's effects, because the append-only file must apply a script whole or not at all, instead of each command part wrapping its own records.
