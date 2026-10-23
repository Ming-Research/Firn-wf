#!/usr/bin/env python3
"""Temporary (firn-scope-blocking-lists, removed before the PR is ready):
records Redis 7.2.16's replies and incremental AOF bytes for list commands that
meet an expired key. Each scenario runs on a fresh redis-server with the AOF
on. A step is one of
  {"pipe": [[arg, ...], ...], "conn": name}   send all in one write, read one reply each
  {"send": [arg, ...], "conn": name}          send, read nothing
  {"read": true, "conn": name}                read one reply
  {"sleep": seconds}
  {"aof": true}                               record the incremental file so far
"""
import glob, json, os, socket, subprocess, sys, tempfile, time

def read_reply(f):
    line = f.readline()
    if not line:
        raise EOFError
    t = line[:1]
    out = line
    if t in b"$=!":
        n = int(line[1:-2])
        if n >= 0:
            out += f.read(n + 2)
    elif t in b"*~>%|":
        n = int(line[1:-2])
        if n >= 0:
            for _ in range(n * 2 if t in b"%|" else n):
                out += read_reply(f)
    return out

def enc(args):
    b = b"*%d\r\n" % len(args)
    for a in args:
        a = a.encode("latin-1")
        b += b"$%d\r\n%s\r\n" % (len(a), a)
    return b

def connect(port):
    for _ in range(200):
        try:
            s = socket.create_connection(("127.0.0.1", port))
            s.settimeout(10)
            return s, s.makefile("rb")
        except OSError:
            time.sleep(0.05)
    raise SystemExit("cannot connect")

SCENARIOS = [
  {"name": "top-level-producers-over-expired-key", "steps": [
    {"pipe": [["SET", "e1", "v", "PXAT", "1"], ["RPUSH", "e1", "b"]], "conn": "p"},
    {"pipe": [["SET", "e2", "v", "PXAT", "1"], ["LPUSH", "e2", "b"]], "conn": "p"},
    {"pipe": [["SET", "e3", "v", "PXAT", "1"], ["RPUSHX", "e3", "b"]], "conn": "p"},
    {"pipe": [["SET", "e4", "v", "PXAT", "1"], ["LINSERT", "e4", "BEFORE", "a", "b"]], "conn": "p"},
    {"pipe": [["RPUSH", "s5", "a"]], "conn": "p"},
    {"pipe": [["SET", "e5", "v", "PXAT", "1"], ["LMOVE", "s5", "e5", "LEFT", "LEFT"]], "conn": "p"},
    {"pipe": [["RPUSH", "s6", "a"]], "conn": "p"},
    {"pipe": [["SET", "e6", "v", "PXAT", "1"], ["RPOPLPUSH", "s6", "e6"]], "conn": "p"},
    {"pipe": [["RPUSH", "r1", "a"]], "conn": "p"},
    {"pipe": [["SET", "e7", "v", "PXAT", "1"], ["RENAME", "r1", "e7"]], "conn": "p"},
    {"pipe": [["RPUSH", "c1", "a"]], "conn": "p"},
    {"pipe": [["SET", "e8", "v", "PXAT", "1"], ["COPY", "c1", "e8"]], "conn": "p"},
    {"aof": True},
  ], "conns": ["p"]},
  {"name": "immediate-blocking-pops-over-expired-key", "steps": [
    {"pipe": [["RPUSH", "l1", "x"]], "conn": "p"},
    {"pipe": [["SET", "e1", "v", "PXAT", "1"], ["BLPOP", "e1", "l1", "0"]], "conn": "p"},
    {"pipe": [["RPUSH", "s1", "a"]], "conn": "p"},
    {"pipe": [["SET", "e2", "v", "PXAT", "1"], ["BRPOPLPUSH", "s1", "e2", "0"]], "conn": "p"},
    {"pipe": [["RPUSH", "s2", "a"]], "conn": "p"},
    {"pipe": [["SET", "e3", "v", "PXAT", "1"], ["BLMOVE", "s2", "e3", "LEFT", "RIGHT", "0"]], "conn": "p"},
    {"aof": True},
  ], "conns": ["p"]},
  {"name": "served-waiter-over-expired-destination", "steps": [
    {"send": ["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"], "conn": "a"},
    {"sleep": 0.3},
    {"pipe": [["SET", "d", "v", "PXAT", "1"], ["RPUSH", "s", "w"]], "conn": "p"},
    {"read": True, "conn": "a"},
    {"aof": True},
  ], "conns": ["p", "a"]},
]

def run(server, scenario, port):
    d = tempfile.mkdtemp()
    p = subprocess.Popen([server, "--port", str(port), "--save", "", "--dir", d,
                          "--appendonly", "yes", "--aof-use-rdb-preamble", "no"], stdout=subprocess.DEVNULL)
    conns = {}
    for name in scenario["conns"]:
        conns[name] = connect(port)
    out = {"name": scenario["name"], "steps": []}
    for step in scenario["steps"]:
        if "sleep" in step:
            time.sleep(step["sleep"]); continue
        if step.get("aof"):
            time.sleep(0.5)
            incr = sorted(glob.glob(os.path.join(d, "appendonlydir", "*.incr.aof")))
            out["steps"].append({"aof": open(incr[-1], "rb").read().decode("latin-1")}); continue
        s, f = conns[step["conn"]]
        if "pipe" in step:
            s.sendall(b"".join(enc(c) for c in step["pipe"]))
            replies = [read_reply(f).decode("latin-1") for _ in step["pipe"]]
            out["steps"].append({"conn": step["conn"], "pipe": step["pipe"], "replies": replies})
        elif "send" in step:
            s.sendall(enc(step["send"]))
            out["steps"].append({"conn": step["conn"], "send": step["send"]})
        elif step.get("read"):
            out["steps"].append({"conn": step["conn"], "read": read_reply(f).decode("latin-1")})
    p.terminate(); p.wait()
    return out

server = sys.argv[1]
results = []
for i, sc in enumerate(SCENARIOS):
    results.append(run(server, sc, 7800 + i))
json.dump(results, open("tmp-oracle-record.json", "w"), indent=1)
for r in results:
    print("== " + r["name"])
    for st in r["steps"]:
        print("  %r" % (st,))
