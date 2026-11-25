# Sorted-set algebra and ZMPOP throughput

This record belongs to item 171, which adds ZUNIONSTORE, ZINTERSTORE, ZDIFFSTORE,
ZUNION, ZINTER, ZDIFF, ZINTERCARD, ZRANGESTORE, ZRANDMEMBER and ZMPOP to firn.
Its criteria and its definitions stand here before any measurement; the
results are added below as the runs finish, each with the run that made it.

## The question

Does firn run the three workloads below at least 1.4 times as fast as the
fastest of Redis, Valkey, Dragonfly and Garnet at depth 16, no slower at depth
1, with no higher p99, on 2 server CPUs; and did the commands' arrival change
the other commands' rates? The first needs a client of its own, since
`redis-benchmark` has no test for these commands.

## Criteria, stated before measuring

These are the acceptance criteria of item 171, with the medians and the twin
spelled out as the firn investigation's
[criteria](DESIGN.md#criteria-stated-before-measuring) define them.

- **Workloads.** The three cells of the table below, RESP2 over loopback, on 2
  server CPUs, with 50 connections each keeping 16 commands unanswered (depth
  16) or 1 (depth 1). The servers run on P-cores and the client on all 16
  E-cores, CPUs 16-31, for every line alike (AGENTS.md).
- **Rate.** At depth 16 firn's rate is at least 1.4 times the fastest
  competitor's; at depth 1 at least the fastest competitor's. Where the client
  is the limit, that is where the median over passes of the client's work
  fraction is above 0.90 in firn's line or in the fastest competitor's, a cell
  asks only that firn is not behind.
- **Latency.** Firn's p99 is no higher than the p99 of the fastest competitor
  of the cell (the one with the highest median rate, the one the rate is
  compared with); a client-limited cell reports it and does not judge it.
- **Layouts.** ZUNIONSTORE and ZINTERSTORE are measured in two layouts and each
  is judged by every criterion above: one pair of source sets that all
  connections read (`shared`, the literal reading of the item), and one pair
  for each connection (`per-connection`, which measures the operation alone).
  Both are judged because the choice between them was put to the owner as a
  question with that option recommended, and the work follows the
  recommendation until the owner answers. A `shared` cell that firn misses
  because it serializes the connections on the shared keys is reported with the
  measurement as a gap of firn or of Whitefoot, never hidden by judging the
  other layout alone.
- **Competitors.** As in the [publish/subscribe record](pubsub-throughput.md#criteria-stated-before-measuring):
  the newest stable release of each project (`redis-latest`, `valkey-latest`
  and `valkey-io-latest`, `dragonfly-latest`, `garnet-latest`), built or
  extracted by `latest.sh` on the run, each with its defaults on the server's 2
  CPUs apart from thread counts. `reference` stays Redis 7.2.16 and is measured
  too. The fastest competitor of a cell is the line with the highest median
  rate in it, `reference` included. A competitor that answers one of the
  commands with an error is not skipped: the run fails, naming the line, so
  that no cell is judged without it unnoticed.
- **Noise.** Firn is measured as two images of one build, `head` and
  `head-twin` (`workload_images`); the 95% interval of the mean log of
  twin/head over the passes is the cell's noise, which `summary.py zset`
  prints. At least 6 interleaved passes, each in the reverse order of the one
  before, give each cell's median.
- **What rejects.** A cell below its threshold stands as measured, with its gap
  and a profile before any cause is named. The cause is then either firn's own,
  fixed in firn, or a gap of Whitefoot, filed with Whitefoot; no safety check
  is removed for speed (constitution, article 2).

## Definitions

Fixed here, and implemented by the workloads `zunionstore`, `zinterstore` and
`zmpop` of `firn-workload`
([source](../../experiments/redis-bench/workload/src/zset.rs)), which
`redis-bench.sh workloads` runs ([header](../../experiments/redis-bench/redis-bench.sh)).

| Cell | Command each connection sends | Sets, written before the run |
|---|---|---|
| `zunionstore-shared` | `ZUNIONSTORE zd:<c> 2 za zb WEIGHTS 1 2` | `za` holds `m0`..`m999` at scores 0..999 and `zb` holds `m500`..`m1499` at twice the member's index: 1000 members each, 500 in both, so the union holds 1500 and the intersection 500; one pair for the run |
| `zunionstore-per-connection` | `ZUNIONSTORE zd:<c> 2 za:<c> zb:<c> WEIGHTS 1 2` | the same two sets for every connection `<c>` |
| `zinterstore-shared`, `zinterstore-per-connection` | the same with `ZINTERSTORE` | as above |
| `zmpop` | `ZMPOP 2 zp:<i> zp:<j> MIN COUNT 1`, `<i>` and `<j>` two distinct numbers drawn at random from 0..99 for each command | 100 sets `zp:<n>`, each of `--prefill` members (default 200000) `m0`..`m<prefill-1>` at their index |

- The destination `zd:<c>` always belongs to the connection `<c>`: the layouts
  differ in the sources only.
- The sets are written with pipelined `ZADD` of 500 pairs a command over a
  connection of the client's own that closes before any measured connection
  opens, outside the measured span; the same call deletes what an earlier run
  left in those keys. Every line gets the same commands.
- The unit is commands a second, which for ZMPOP with `COUNT 1` is members
  popped a second. A batch's latency runs from its first byte written to its
  last reply read; the percentiles are over batches (at depth 1, commands).
- Each reply is checked, and a failed check fails the run for every server
  alike: ZUNIONSTORE must answer 1500, ZINTERSTORE 500, ZMPOP an array of the
  key and exactly `--pop-count` pairs, so that an empty (null) reply, which
  would mean the prefill was too small for the run, is a failure and no rate.
  The prefill must outlast the run: the `zmpop` cells run `WORKLOAD_SECONDS`
  3, at no more than 5 million commands a second that is 15 million pops, 150
  thousand a key over 100 keys, which 200000 holds with a third to spare. A
  pilot of 2 passes shows whether it does; the prefill is raised, for every
  line alike, if any line fails the pilot.
- A cell with `--pop-count 10` is recorded once the `COUNT 1` cells are judged,
  with a prefill ten times as large and the run shortened as much; it is not
  judged.

### Whether a shared pair serializes the connections

Before the `shared` cells are judged, one run says whether connections that read
the same two keys wait for each other in firn. A statement over key entries
holds them exclusively and a lookup writes the entry's access stamp, so this
is a prediction from the code and is checked here, not assumed:

- `workloads` mode, `workload_servers` `firn`, `cpus` 2, `tests`
  `zunionstore`, `connections` `1 2`, `depths` 16, 6 passes of 5 seconds.
- For each layout, the rate at 2 connections over the rate at 1. The
  `per-connection` layout is the control: two connections on two server cores
  that share nothing.
- The prediction is that `shared` gives a ratio near 1 and `per-connection` a
  ratio near 2. The prediction is rejected, and reads of one key do not
  serialize, if the `shared` ratio is at least 0.9 times the `per-connection`
  ratio.

## The 20 default items

The criterion is the change from before the commands to after them, not an
absolute rate. The 20 tests of `redis-benchmark`'s default suite
(`SUITE_DEFAULT` in `redis-bench.sh`) at depth 16 on 2 server CPUs: `scale`
mode with `cpus` 2, `tests` the suite's 20 items and `workload_servers` naming
the five `-latest` servers, which makes the run fetch them
(`SCALE_PIPELINES` is not an input, so the depth is 16; these items have no
depth-1 criterion). One interleaved run measures `firn` (head), `firn-base`
(the revision before this item, from `revisions`) and the competitors, so each
item has firn's and base's multiple of the fastest competitor in the same
passes. The noise is a `compare` run of `base head head-twin` over the same 20
items at depth 16, computed by `summary.py ratios`.

The criterion: for each item, head's multiple of the fastest competitor over
base's multiple is within the head-twin/head interval of the compare run, which
is the noise of the instrument; an item outside it is a slowdown or a gain to
explain before this item merges. The 20 items never touch a new command; the
change they can show is the shared dispatch's, which this item grew by ten
names.

## Results

No measurement has been made. The cells, the exclusivity check and the 20 items
are added here, each with the run that made it and its tables copied in, as the
runs finish.
