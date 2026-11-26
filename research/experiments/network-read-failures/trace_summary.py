#!/usr/bin/env python3
"""Summarises a kernel trace of SIGCHLD for the processes named.

    trace_summary.py --pids FILE --label TEXT TRACE

TRACE is the text of /sys/kernel/tracing/trace with the option record-tgid and the events signal:signal_generate and
signal:signal_deliver (both filtered to sig == 17) and task:task_newtask, as run.sh beside this file records them. FILE
holds the process ids (thread group ids) to report on, one a line.

It prints, for those processes: how many SIGCHLD were discarded, queued or found already pending as they were generated
(the tracepoint's res: 1, 0 and 2), which children's exits were queued, and which threads dequeued a SIGCHLD. A thread
that dequeues one was woken for it: had it been waiting in a socket call with a timeout, that call would have failed
with EINTR.
"""

import argparse
import collections
import re
import sys

LINE = re.compile(
    r"^\s*(?P<comm>.+?)-(?P<pid>\d+)\s+\(\s*(?P<tgid>[-\d]+)\)\s+\[\d+\]\s+\S+\s+(?P<time>[\d.]+): "
    r"(?P<event>signal_generate|signal_deliver|task_newtask): (?P<rest>.*)$"
)
GENERATE = re.compile(r"sig=17 .*comm=(?P<comm>.*) pid=(?P<pid>\d+) grp=\d+ res=(?P<res>\d+)$")
NEWTASK = re.compile(r"pid=(?P<pid>\d+) comm=(?P<comm>.*) clone_flags=(?P<flags>[0-9a-f]+)")
CLONE_THREAD = 0x10000
RESULTS = {0: "queued", 1: "discarded", 2: "already_pending"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pids", required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--examples", type=int, default=12)
    parser.add_argument("trace")
    args = parser.parse_args()

    with open(args.pids, encoding="utf-8") as given:
        named = {int(line) for line in given if line.strip().isdigit()}

    events = []
    unparsed = 0
    with open(args.trace, encoding="utf-8", errors="replace") as trace:
        for line in trace:
            if line.startswith("#") or not line.strip():
                continue
            found = LINE.match(line.rstrip("\n"))
            if found is None:
                unparsed += 1
                continue
            events.append(found)

    # Which process each thread belongs to: from the trace's own column, and from the creation of each task.
    group = {}
    for event in events:
        pid = int(event["pid"])
        if event["tgid"].isdigit():
            group[pid] = int(event["tgid"])
    for event in events:
        if event["event"] != "task_newtask":
            continue
        new = NEWTASK.match(event["rest"])
        if new is None:
            continue
        creator = group.get(int(event["pid"]))
        child = int(new["pid"])
        if int(new["flags"], 16) & CLONE_THREAD:
            if creator is not None:
                group.setdefault(child, creator)
        else:
            group.setdefault(child, child)

    generated = collections.Counter()
    queued_children = collections.Counter()
    dequeued_first = 0
    dequeued_others = collections.Counter()
    examples = []
    last_queued = {}
    for event in events:
        pid = int(event["pid"])
        if event["event"] == "signal_generate":
            target = GENERATE.match(event["rest"])
            if target is None:
                continue
            owner = group.get(int(target["pid"]))
            if owner not in named:
                continue
            result = int(target["res"])
            generated[RESULTS.get(result, f"res{result}")] += 1
            if result != 1:
                queued_children[event["comm"].strip()] += 1
            if result == 0:
                last_queued[owner] = event.string.rstrip("\n")
        elif event["event"] == "signal_deliver":
            owner = group.get(pid)
            if owner not in named:
                continue
            if pid == owner:
                dequeued_first += 1
            else:
                dequeued_others[event["comm"].strip()] += 1
                if len(examples) < args.examples:
                    examples.append((last_queued.get(owner, "(no queued SIGCHLD seen before it)"), event.string.rstrip("\n")))

    label = args.label
    counts = " ".join(f"{name}={generated[name]}" for name in ("discarded", "queued", "already_pending"))
    others = {name: count for name, count in generated.items() if name not in RESULTS.values()}
    print(f"trace {label}: processes named={len(named)} lines parsed={len(events)} not parsed={unparsed}")
    print(f"trace {label}: SIGCHLD generated for their threads: {counts}{' ' + str(others) if others else ''}")
    print(f"trace {label}: queued or pending, by the exiting child: {dict(queued_children.most_common(12))}")
    print(
        f"trace {label}: SIGCHLD dequeued in them: by the first thread={dequeued_first} "
        f"by other threads={sum(dequeued_others.values())}"
    )
    if dequeued_others:
        print(f"trace {label}: dequeued by other threads, by thread name: {dict(dequeued_others.most_common(20))}")
    for queued, delivered in examples:
        print(f"trace {label}: example queued:   {queued.strip()}")
        print(f"trace {label}: example dequeued: {delivered.strip()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
