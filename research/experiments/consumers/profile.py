"""Reduce a Redis MONITOR record to the forms of the commands it holds.

    python3 profile.py monitor.log > profile.tsv

A form is a command's name, its subcommand for a command that has them, and
the option words it was sent with, in the order sent. The output counts each
form by its source: `client` for a command a client sent, `lua` for one a
script ran. Rows of source `script` are not executions: they name each command
the source text of a script sent with EVAL can call, counted by the distinct
scripts naming it, since a test may leave a script's branch unexercised. An
argument counts as an option word when it spells one of OPTIONS in any case,
so a value that happens to spell one is counted too. The number of commands
read goes to standard error; a record with none exits 1. run.sh calls it.
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
# the command's words, each quoted with backslash escapes. The source has no
# space but may hold brackets itself, as an IPv6 address does: [::1]:5000.
LINE = re.compile(r'^\d+\.\d+ \[\d+ ([^ ]+)\] (.*)$')
WORD = re.compile(r'"((?:[^"\\]|\\.)*)"')
# A command a script's source calls: redis.call('name', ...) or pcall.
CALL = re.compile(r"""redis\.p?call\(\s*['"]([A-Za-z_]+)['"]""")
# The escapes MONITOR writes inside a quoted word, as Redis's sdscatrepr does.
ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "a": "\a", "b": "\b", "\\": "\\", '"': '"'}


def unescape(word):
    """A quoted MONITOR word's text, its escapes decoded."""
    out = []
    i = 0
    while i < len(word):
        if word[i] == "\\" and i + 1 < len(word):
            nxt = word[i + 1]
            if nxt == "x" and i + 3 < len(word):
                out.append(chr(int(word[i + 2:i + 4], 16)))
                i += 4
                continue
            out.append(ESCAPES.get(nxt, nxt))
            i += 2
            continue
        out.append(word[i])
        i += 1
    return "".join(out)


def forms(path):
    counts = Counter()
    scripts = set()
    total = 0
    with open(path, encoding="utf-8", errors="replace") as record:
        for line in record:
            match = LINE.match(line.rstrip("\n"))
            if not match:
                continue
            source = "lua" if match.group(1) == "lua" else "client"
            words = WORD.findall(match.group(2))
            if not words:
                continue
            total += 1
            name = words[0].upper()
            form = [name]
            rest = words[1:]
            if name in CONTAINERS and rest:
                form.append(rest[0].upper())
                rest = rest[1:]
            if name in ("EVAL", "EVAL_RO") and rest:
                scripts.add(rest[0])
            if name in ("EVAL", "EVALSHA", "EVAL_RO", "EVALSHA_RO"):
                rest = []
            if name == "SCRIPT" and len(form) > 1 and form[1] == "LOAD" and rest:
                scripts.add(rest[0])
                rest = []
            form.extend(word.upper() for word in rest if word.upper() in OPTIONS)
            counts[(source, " ".join(form))] += 1
    for script in scripts:
        for name in sorted(set(call.upper() for call in CALL.findall(unescape(script)))):
            counts[("script", name)] += 1
    return counts, total


def main():
    counts, total = forms(sys.argv[1])
    print(f"{total} commands", file=sys.stderr)
    if total == 0:
        print("the record holds no command", file=sys.stderr)
        sys.exit(1)
    print("source\tform\tcount")
    for (source, form), count in sorted(counts.items(), key=lambda item: (-item[1], item[0])):
        print(f"{source}\t{form}\t{count}")


if __name__ == "__main__":
    main()
