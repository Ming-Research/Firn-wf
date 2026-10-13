#!/usr/bin/env python3
"""Temporary (branch claude/oracle-replies, never merged): records Redis
7.2.16's raw replies to command batches, the oracle for firn's expected
replies. Each batch file is JSON: a list of
{"name": str, "resp": 2|3, "config": [[k, v], ...]?, "commands": [[arg, ...], ...],
 "rewrite_aof": bool?}.
Each batch runs on a fresh redis-server; with rewrite_aof the server runs with
appendonly yes and aof-use-rdb-preamble no, then BGREWRITEAOF, and the base
file's bytes are recorded."""
import json, os, socket, subprocess, sys, tempfile, time, glob

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
            k = n * 2 if t in b"%|" else n
            for _ in range(k):
                out += read_reply(f)
            if t == b"|":
                out += read_reply(f)
    return out

def enc(args):
    b = b"*%d\r\n" % len(args)
    for a in args:
        a = a.encode("latin-1") if isinstance(a, str) else bytes(a)
        b += b"$%d\r\n%s\r\n" % (len(a), a)
    return b

def start(server, args, port, resp):
    p = subprocess.Popen(args, stdout=subprocess.DEVNULL)
    for _ in range(200):
        try:
            s = socket.create_connection(("127.0.0.1", port)); break
        except OSError:
            time.sleep(0.05)
    f = s.makefile("rb")
    while True:
        s.sendall(enc(["PING"]))
        if not read_reply(f).startswith(b"-LOADING"):
            break
        time.sleep(0.05)
    if resp == 3:
        s.sendall(enc(["HELLO", "3"])); read_reply(f)
    return p, s, f

def rewrite(s, f, d):
    s.sendall(enc(["BGREWRITEAOF"])); read_reply(f)
    for _ in range(400):
        s.sendall(enc(["INFO", "persistence"])); info = read_reply(f).decode()
        if "aof_rewrite_in_progress:0" in info and "aof_rewrite_scheduled:0" in info:
            break
        time.sleep(0.05)
    time.sleep(0.2)
    bases = sorted(glob.glob(os.path.join(d, "appendonlydir", "*.base.aof")))
    return open(bases[-1], "rb").read().decode("latin-1") if bases else None

def run(server, batch, port):
    d = tempfile.mkdtemp()
    args = [server, "--port", str(port), "--save", "", "--dir", d]
    if batch.get("rewrite_aof") or batch.get("appendonly"):
        args += ["--appendonly", "yes", "--aof-use-rdb-preamble", "no", "--auto-aof-rewrite-percentage", "0"]
    cfg = batch.get("config") or []
    if isinstance(cfg, dict):
        cfg = list(cfg.items())
    for k, v in cfg:
        args += ["--" + k, v]
    resp = batch.get("resp", 2)
    p, s, f = start(server, args, port, resp)
    res = {"name": batch["name"], "resp": resp, "replies": []}
    for c in batch["commands"]:
        if c == ["__RESTART__"]:
            s.sendall(enc(["SHUTDOWN"])); s.close(); p.wait()
            p, s, f = start(server, args, port, resp)
            res["replies"].append({"cmd": c, "reply": ""}); continue
        if c == ["__AOF__"]:
            time.sleep(1.2)
            incr = sorted(glob.glob(os.path.join(d, "appendonlydir", "*.incr.aof")), key=lambda x: int(x.split(".")[-3]))
            res["replies"].append({"cmd": c, "reply": open(incr[-1], "rb").read().decode("latin-1") if incr else ""}); continue
        if c == ["__REWRITE__"]:
            res["replies"].append({"cmd": c, "reply": rewrite(s, f, d) or ""}); continue
        s.sendall(enc(c))
        r = read_reply(f)
        if batch.get("drain"):
            s.settimeout(0.3)
            try:
                while True:
                    r += read_reply(f)
            except (socket.timeout, TimeoutError, OSError):
                pass
            s.settimeout(None)
            f = s.makefile("rb")
        res["replies"].append({"cmd": c, "reply": r.decode("latin-1")})
    if batch.get("rewrite_aof"):
        res["aof_base"] = rewrite(s, f, d)
    p.terminate(); p.wait()
    return res

server = sys.argv[1]
out = []
port = 7700
for path in sorted(glob.glob("oracle/batches/*.json")):
    for b in json.load(open(path)):
        port += 1
        r = run(server, b, port); r["file"] = os.path.basename(path); out.append(r)
json.dump(out, open("oracle-replies.json", "w"), indent=1)
for r in out:
    print("== %s :: %s (RESP%d)" % (r["file"], r["name"], r["resp"]))
    for x in r["replies"]:
        print("  %r -> %r" % (x["cmd"], x["reply"]))
    if "aof_base" in r:
        print("  AOF base: %r" % r["aof_base"])
