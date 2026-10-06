"""Reduce a Redis MONITOR record to the forms of the commands it holds.

    python3 profile.py monitor.log > profile.tsv

A form is a command's name, its subcommand for a command that has them, and
the option words it was sent with, in the order sent; the output counts each
form by its source, a client or a script (`lua`). An argument counts as an
option word when it spells one of OPTIONS in any case, so a value that
happens to spell one is counted too; the profile bounds what a consumer
sends rather than parsing each command's syntax. run.sh calls it.
"""

import re
import sys
from collections import Counter

# Commands whose first argument names a subcommand.
CONTAINERS = {
    "ACL", "CLIENT", "CLUSTER", "COMMAND", "CONFIG", "DEBUG", "FUNCTION",
    "LATENCY", "MEMORY", "MODULE", "OBJECT", "PUBSUB", "SCRIPT", "SLOWLOG",
    "XGROUP", "XINFO",
}

OPTIONS = {
    "ABSTTL", "AGGREGATE", "ALPHA", "ASC", "ASYNC", "BLOCK", "BY", "BYLEX",
    "BYSCORE", "CH", "COPY", "COUNT", "DB", "DESC", "EX", "EXAT", "FIELDS",
    "FLUSH", "FREQ", "GET", "GT", "IDLETIME", "INCR", "KEEPTTL", "KEYS",
    "LEFT", "LEN", "LIMIT", "LT", "MATCH", "MAX", "MAXLEN", "MIN", "NOVALUES",
    "NX", "PERSIST", "PX", "PXAT", "RANK", "REPLACE", "REV", "RIGHT",
    "SETNAME", "STORE", "SYNC", "TYPE", "WEIGHTS", "WITHCOUNT", "WITHSCORE",
    "WITHSCORES", "WITHVALUES", "XX",
}

# A MONITOR line: a timestamp, the database and the source in brackets, then
# the command's words, each quoted with backslash escapes.
LINE = re.compile(r'^\d+\.\d+ \[\d+ ([^\]]+)\] (.*)$')
WORD = re.compile(r'"((?:[^"\\]|\\.)*)"')


def forms(path):
    counts = Counter()
    with open(path, encoding="utf-8", errors="replace") as record:
        for line in record:
            match = LINE.match(line.rstrip("\n"))
            if not match:
                continue
            source = "lua" if match.group(1) == "lua" else "client"
            words = WORD.findall(match.group(2))
            if not words:
                continue
            name = words[0].upper()
            form = [name]
            rest = words[1:]
            if name in CONTAINERS and rest:
                form.append(rest[0].upper())
                rest = rest[1:]
            if name in ("EVAL", "EVALSHA", "EVAL_RO", "EVALSHA_RO"):
                rest = []
            form.extend(word.upper() for word in rest if word.upper() in OPTIONS)
            counts[(source, " ".join(form))] += 1
    return counts


def main():
    counts = forms(sys.argv[1])
    print("source\tform\tcount")
    for (source, form), count in sorted(counts.items(), key=lambda item: (-item[1], item[0])):
        print(f"{source}\t{form}\t{count}")


if __name__ == "__main__":
    main()
