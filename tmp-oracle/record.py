#!/usr/bin/env python3
"""Temporary (firn-scope-blocking-lists, removed before the PR is ready):
records Redis 7.2.16's replies and incremental AOF bytes for list writes
while a blocked client waits on a key that holds a string. Each scenario runs on a fresh redis-server with the AOF
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
  {"name": "waiter-survives-an-expired-string-then-push", "steps": [
    {"send": ["BLPOP", "k", "0"], "conn": "a"},
    {"sleep": 0.3},
    {"pipe": [["SET", "k", "v", "PXAT", "1"], ["LPUSH", "k", "x"]], "conn": "p"},
    {"read": True, "conn": "a"},
    {"aof": True},
  ], "conns": ["p", "a"]},
  {"name": "waiter-survives-an-expired-string-then-move", "steps": [
    {"send": ["BLPOP", "d", "0"], "conn": "a"},
    {"sleep": 0.3},
    {"pipe": [["RPUSH", "s", "w"], ["SET", "d", "v", "PXAT", "1"], ["LMOVE", "s", "d", "LEFT", "LEFT"]], "conn": "p"},
    {"read": True, "conn": "a"},
    {"aof": True},
  ], "conns": ["p", "a"]},
  {"name": "waiter-survives-a-live-string-and-a-refused-push", "steps": [
    {"send": ["BLPOP", "k", "0"], "conn": "a"},
    {"sleep": 0.3},
    {"pipe": [["SET", "k", "v"], ["LPUSH", "k", "x"], ["LPUSHX", "k", "x"], ["RPUSH", "k", "x"]], "conn": "p"},
    {"sleep": 0.3},
    {"pipe": [["DEL", "k"], ["LPUSH", "k", "y"]], "conn": "p"},
    {"read": True, "conn": "a"},
    {"aof": True},
  ], "conns": ["p", "a"]},
  {"name": "pushes-onto-a-live-list-with-a-waiter-elsewhere", "steps": [
    {"send": ["BLPOP", "w", "0"], "conn": "a"},
    {"sleep": 0.3},
    {"pipe": [["RPUSH", "l", "a"], ["LPUSH", "l", "b"], ["LPUSHX", "l", "c"], ["RPUSHX", "l", "d"], ["LINSERT", "l", "BEFORE", "a", "e"], ["LMOVE", "l", "l", "LEFT", "RIGHT"], ["RPOPLPUSH", "l", "l"], ["LRANGE", "l", "0", "-1"]], "conn": "p"},
    {"pipe": [["LPUSH", "w", "z"]], "conn": "p"},
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
