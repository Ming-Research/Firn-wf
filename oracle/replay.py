#!/usr/bin/env python3
"""Temporary Redis 7.2.16 wire/AOF recorder (claude/oracle-replies only).

A file contains a list of batches. Each batch gets a fresh server and directory.
The original form is unchanged: {name, resp: 2|3 (default 2), commands: [argv],
config: [[key, value]]|object, appendonly: bool, rewrite_aof: bool, drain: bool}.
Arguments are Latin-1 strings or integer byte arrays. Legacy __AOF__, __REWRITE__
and __RESTART__ commands and the replies/cmd/reply output are retained.

The second form replaces commands with operations, an ordered list of objects:
  {op: "open", client: "A", resp: 2|3}       # default batch resp
  {op: "close", client: "A", how: "both"|"write"|"read"} # default both
  {op: "send", client: "A", argv: [...] }   # never reads
  {op: "pipeline", client: "A", commands: [[...], ...]} # one sendall
  {op: "read", client: "A", count: N, timeout_ms: 5000, capture: "id",
   allow_eof: false}
  {op: "quiet", client: "A", ms: 100}       # no complete frame during interval
  {op: "poll_info", client: "P", section: "clients", field: "blocked_clients",
   equals: 1, timeout_ms: 5000, interval_ms: 20, record_fields: ["other_field"]}
  {op: "aof"}                              # raw incremental files after 1.2 s
  {op: "rewrite", client: "P", timeout_ms: 30000} # BGREWRITEAOF + raw base
  {op: "restart", timeout_ms: 10000}        # closes clients; reopen explicitly
Read defaults to one physical frame, including pushes. RESP3 attributes attach
 to their following frame. Capture requires one integer, simple or bulk scalar;
its lossless byte value can replace a later argv operand as {"var": "id"}.
Read with allow_eof true stops at count frames or clean socket EOF, whichever
comes first, recording eof:true and any preceding frames. EOF with an incomplete
buffer fails. This option cannot be combined with capture. Default reads still
require exactly count frames; both modes retain their harness deadline.
Nulls, errors and aggregates cannot be captured. INFO records each polling
exchange, complete raw reply bytes, selected fields, and (for clients) all three
blocking metrics. Missing fields do not equal a requested value.

Every operation records client (null for server-only operations), sent/received
Latin-1 wire bytes, consumed frames with reply/push kind, elapsed_ms, and raw
socket_received bytes (which may include read-ahead). Bytes not yet consumed
remain in that client's persistent buffer. quiet never discards partial data.
On a failed sendall, attempted_sent records the requested bytes and
sent_complete is false; Python cannot expose how many bytes reached the socket.
Frames include elapsed_ms_since_send from the client's latest send; operations
include started_ms measured from batch start. Open records HELLO 3 when selected.
AOF/rewrite record exact Latin-1 file bytes. Extra batch metadata is retained.
Per-read/start/send/rewrite/shutdown deadlines are capped by batch timeout_ms
(default 120000). Failures retain the failed operation and preceding evidence,
write oracle-replies.json and exit nonzero; finally closes sockets and kills the
server. Timings are observations, not inferred compatibility expectations.
"""
import glob
import json
import math
import os
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import time


class Incomplete(Exception):
    pass


def milliseconds(value, name, allow_zero=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError('%s must be a finite number' % name)
    if not math.isfinite(value) or value < 0 or (value == 0 and not allow_zero):
        raise ValueError('%s must be %s' % (name, 'nonnegative' if allow_zero else 'positive'))
    return value / 1000


def scan(data, pos=0):
    """Return (end, actual type, scalar bytes) without consuming incomplete data."""
    endline = data.find(b'\r\n', pos)
    if endline < 0:
        raise Incomplete
    tag = bytes(data[pos:pos + 1])
    body = bytes(data[pos + 1:endline])
    end = endline + 2
    if tag in (b'+', b'-', b':', b',', b'(', b'#', b'_'):
        return end, tag, body if tag in (b'+', b':') else None
    if tag in (b'$', b'=', b'!'):
        size = int(body)
        if size < -1:
            raise ValueError('invalid RESP length')
        if size == -1:
            return end, tag, None
        stop = end + size
        if len(data) < stop + 2:
            raise Incomplete
        if data[stop:stop + 2] != b'\r\n':
            raise ValueError('invalid RESP payload terminator')
        return stop + 2, tag, bytes(data[end:stop]) if tag == b'$' else None
    if tag in (b'*', b'~', b'>', b'%', b'|'):
        size = int(body)
        if size < -1:
            raise ValueError('invalid RESP aggregate length')
        if size >= 0:
            for _ in range(size * (2 if tag in (b'%', b'|') else 1)):
                end, _, _ = scan(data, end)
        if tag == b'|':
            return scan(data, end)
        return end, tag, None
    raise ValueError('unknown RESP type %r' % tag)


def enc(args, variables=None):
    if not isinstance(args, list):
        raise ValueError('argv must be a list')
    out = b'*%d\r\n' % len(args)
    for arg in args:
        if isinstance(arg, dict) and set(arg) == {'var'}:
            arg = (variables or {})[arg['var']]
        if isinstance(arg, str):
            arg = arg.encode('latin-1')
        elif isinstance(arg, list):
            if any(isinstance(x, bool) or not isinstance(x, int) or not 0 <= x <= 255 for x in arg):
                raise ValueError('byte-array operands require integers in 0..255')
            arg = bytes(arg)
        elif not isinstance(arg, bytes):
            raise ValueError('argv operands must be Latin-1 strings, byte arrays or variable references')
        out += b'$%d\r\n%s\r\n' % (len(arg), arg)
    return out


def trace_for(op, client=None):
    return {'op': op, 'client': client, 'sent': '', 'received': '', 'socket_received': '', 'frames': []}


def time_left(deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError('harness deadline exceeded')
    return remaining


class Client:
    def __init__(self, sock):
        self.sock = sock
        self.sock.setblocking(False)
        self.buffer = bytearray()
        self.last_send = None

    def close(self):
        self.sock.close()

    def send(self, wire, deadline, trace):
        self.sock.settimeout(time_left(deadline))
        self.last_send = time.monotonic()
        try:
            self.sock.sendall(wire)
        except Exception:
            # sendall does not expose its partial-write count after a failure.
            trace['attempted_sent'] = wire.decode('latin-1')
            trace['sent_complete'] = False
            raise
        else:
            trace['sent'] += wire.decode('latin-1')
        finally:
            self.sock.setblocking(False)

    def peek(self):
        try:
            end, tag, scalar = scan(self.buffer)
        except Incomplete:
            return None
        return end, tag, scalar

    def receive(self, deadline, trace):
        ready, _, _ = select.select([self.sock], [], [], time_left(deadline))
        if not ready:
            raise TimeoutError('frame read exceeded harness deadline')
        chunk = self.sock.recv(65536)
        if not chunk:
            raise EOFError('client closed before a complete frame')
        trace['socket_received'] += chunk.decode('latin-1')
        self.buffer.extend(chunk)

    def read(self, deadline, trace):
        while True:
            time_left(deadline)
            frame = self.peek()
            if frame is not None:
                end, tag, scalar = frame
                raw = bytes(self.buffer[:end])
                del self.buffer[:end]
                wire = raw.decode('latin-1')
                trace['received'] += wire
                frame_trace = {'bytes': wire, 'kind': 'push' if tag == b'>' else 'reply'}
                if self.last_send is not None:
                    frame_trace['elapsed_ms_since_send'] = round((time.monotonic() - self.last_send) * 1000, 3)
                trace['frames'].append(frame_trace)
                return raw, tag, scalar
            self.receive(deadline, trace)

    def reply(self, deadline, trace):
        # Preserve pushes in the operation trace while locating the reply.
        while True:
            frame = self.read(deadline, trace)
            if frame[1] != b'>':
                return frame

    def quiet(self, deadline, trace):
        while True:
            if self.peek() is not None:
                raise AssertionError('complete frame arrived during quiet interval')
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            ready, _, _ = select.select([self.sock], [], [], remaining)
            if not ready:
                return
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError('client closed during quiet interval')
            trace['socket_received'] += chunk.decode('latin-1')
            self.buffer.extend(chunk)


def info_fields(raw):
    _, tag, body = scan(raw)
    if tag == b'=' and body is not None and body[3:4] == b':':
        body = body[4:]  # RESP3 verbatim string: drop the 'txt:' format prefix
    elif tag != b'$' or body is None:
        raise ValueError('INFO did not return a bulk or verbatim string: %r' % raw)
    fields = {}
    for line in body.split(b'\r\n'):
        if b':' in line and not line.startswith(b'#'):
            key, value = line.split(b':', 1)
            fields[key.decode('latin-1')] = value.decode('latin-1')
    return fields


class Runner:
    def __init__(self, server, batch, port):
        self.batch = batch
        self.port = port
        self.started = time.monotonic()
        self.deadline = self.started + milliseconds(batch.get('timeout_ms', 120000), 'batch timeout_ms')
        cfg = batch.get('config') or []
        if isinstance(cfg, dict):
            cfg = cfg.items()
        config_args = []
        for key, value in cfg:
            config_args += ['--' + key, str(value)]
        self.directory = tempfile.mkdtemp(prefix='redis-oracle-')
        self.clients = {}
        self.variables = {}
        self.process = None
        self.args = [server, '--port', str(port), '--save', '', '--dir', self.directory]
        if batch.get('rewrite_aof') or batch.get('appendonly'):
            self.args += ['--appendonly', 'yes', '--aof-use-rdb-preamble', 'no', '--auto-aof-rewrite-percentage', '0']
        self.args += config_args

    def limit(self, ms=5000):
        return min(self.deadline, time.monotonic() + milliseconds(ms, 'timeout_ms'))

    def pause(self, seconds, deadline=None):
        deadline = min(self.deadline, deadline if deadline is not None else self.deadline)
        if time_left(deadline) < seconds:
            raise TimeoutError('wait would exceed harness deadline')
        time.sleep(seconds)

    def connect(self, deadline):
        time_left(deadline)
        sock = socket.create_connection(('127.0.0.1', self.port), timeout=time_left(deadline))
        return Client(sock)

    def start(self, deadline):
        self.process = subprocess.Popen(self.args, stdout=subprocess.DEVNULL)
        probe = None
        try:
            while probe is None:
                time_left(deadline)
                if self.process.poll() is not None:
                    raise RuntimeError('Redis exited during startup with status %s' % self.process.returncode)
                try:
                    probe = self.connect(deadline)
                except OSError:
                    self.pause(0.05, deadline)
            trace = trace_for('startup', '__startup__')
            while True:
                probe.send(enc(['PING']), deadline, trace)
                raw, _, _ = probe.read(deadline, trace)
                if raw == b'+PONG\r\n':
                    return
                if not raw.startswith(b'-LOADING'):
                    raise RuntimeError('unexpected startup reply %r' % raw)
                self.pause(0.05, deadline)
        finally:
            if probe is not None:
                probe.close()

    def open(self, name, resp, deadline, trace):
        if not isinstance(name, str) or not name:
            raise ValueError('client name must be a nonempty string')
        if name in self.clients:
            raise ValueError('client already open: ' + name)
        if isinstance(resp, bool) or resp not in (2, 3):
            raise ValueError('resp must be 2 or 3')
        client = self.connect(deadline)
        self.clients[name] = client
        if resp == 3:
            client.send(enc(['HELLO', '3']), deadline, trace)
            _, tag, _ = client.read(deadline, trace)
            if tag in (b'-', b'!'):
                raise RuntimeError('HELLO failed')
        return client

    def shutdown(self, deadline, trace=None):
        trace = trace if trace is not None else trace_for('shutdown', '__shutdown__')
        control = None
        try:
            if self.process is not None and self.process.poll() is None:
                control = self.connect(deadline)
                control.send(enc(['SHUTDOWN']), deadline, trace)
                # SHUTDOWN success closes the connection; it has no reply.
                while True:
                    ready, _, _ = select.select([control.sock], [], [], time_left(deadline))
                    if not ready:
                        raise TimeoutError('shutdown exceeded harness deadline')
                    chunk = control.sock.recv(65536)
                    if not chunk:
                        trace['eof'] = True
                        break
                    trace['socket_received'] += chunk.decode('latin-1')
                    trace['received'] += chunk.decode('latin-1')
                    control.buffer.extend(chunk)
                    if control.peek() is not None:
                        end, _, _ = control.peek()
                        trace['frames'].append({'bytes': bytes(control.buffer[:end]).decode('latin-1'), 'kind': 'reply'})
                        raise RuntimeError('SHUTDOWN returned a reply instead of closing the connection')
                self.process.wait(timeout=time_left(deadline))
        finally:
            if control is not None:
                control.close()
            for client in self.clients.values():
                client.close()
            self.clients.clear()
            if self.process is not None and self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=2)

    def cleanup(self):
        for client in self.clients.values():
            client.close()
        self.clients.clear()
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=2)
        shutil.rmtree(self.directory)

    def aof(self):
        self.pause(1.2)
        paths = sorted(glob.glob(os.path.join(self.directory, 'appendonlydir', '*.incr.aof')),
                       key=lambda path: int(path.split('.')[-3]))
        return [{'name': os.path.basename(path), 'bytes': self.file_bytes(path)} for path in paths]

    @staticmethod
    def file_bytes(path):
        with open(path, 'rb') as source:
            return source.read().decode('latin-1')

    def rewrite(self, client, deadline, trace):
        client.send(enc(['BGREWRITEAOF']), deadline, trace)
        _, tag, _ = client.reply(deadline, trace)
        if tag in (b'-', b'!'):
            raise RuntimeError('BGREWRITEAOF failed')
        while True:
            client.send(enc(['INFO', 'persistence']), deadline, trace)
            raw, _, _ = client.reply(deadline, trace)
            fields = info_fields(raw)
            if fields.get('aof_rewrite_in_progress') == '0' and fields.get('aof_rewrite_scheduled') == '0':
                if fields.get('aof_last_bgrewrite_status') != 'ok':
                    raise RuntimeError('AOF rewrite did not succeed')
                break
            self.pause(0.05, deadline)
        self.pause(0.2, deadline)
        paths = sorted(glob.glob(os.path.join(self.directory, 'appendonlydir', '*.base.aof')),
                       key=lambda path: int(path.split('.')[-3]))
        return self.file_bytes(paths[-1]) if paths else None

    def operation(self, op, trace):
        kind = op['op']
        name = op.get('client')
        deadline = self.limit(op.get('timeout_ms', 30000 if kind == 'rewrite' else 10000 if kind == 'restart' else 5000))
        if kind == 'open':
            trace['resp'] = op.get('resp', self.batch.get('resp', 2))
            self.open(name, trace['resp'], deadline, trace)
        elif kind == 'restart':
            self.shutdown(deadline, trace)
            self.start(deadline)
        elif kind == 'aof':
            trace['files'] = self.aof()
        else:
            client = self.clients[name]
            if kind == 'close':
                how = op.get('how', 'both')
                if how == 'both':
                    client.close()
                    del self.clients[name]
                elif how in ('read', 'write'):
                    client.sock.shutdown(socket.SHUT_RD if how == 'read' else socket.SHUT_WR)
                else:
                    raise ValueError('close how must be both, read or write')
            elif kind in ('send', 'pipeline'):
                commands = [op['argv']] if kind == 'send' else op['commands']
                if not isinstance(commands, list):
                    raise ValueError('pipeline commands must be a list')
                client.send(b''.join(enc(argv, self.variables) for argv in commands), deadline, trace)
            elif kind == 'read':
                count = op.get('count', 1)
                if isinstance(count, bool) or not isinstance(count, int) or count < 1:
                    raise ValueError('read count must be a positive integer')
                allow_eof = op.get('allow_eof', False)
                if not isinstance(allow_eof, bool):
                    raise ValueError('read allow_eof must be a boolean')
                if allow_eof and 'capture' in op:
                    raise ValueError('capture cannot be combined with allow_eof')
                if 'capture' in op and count != 1:
                    raise ValueError('capture requires read count 1')
                for _ in range(count):
                    try:
                        _, tag, scalar = client.read(deadline, trace)
                    except EOFError:
                        if not allow_eof or client.buffer:
                            raise
                        trace['eof'] = True
                        break
                if 'capture' in op:
                    if scalar is None or tag not in (b':', b'+', b'$'):
                        raise ValueError('capture requires a nonnull integer, simple or bulk string')
                    variable = op['capture']
                    if not isinstance(variable, str) or not variable:
                        raise ValueError('capture must name a nonempty variable')
                    self.variables[variable] = scalar
                    trace['capture'] = {'name': variable, 'bytes': scalar.decode('latin-1')}
            elif kind == 'quiet':
                duration = milliseconds(op['ms'], 'quiet ms', allow_zero=True)
                quiet_deadline = time.monotonic() + duration
                if quiet_deadline > self.deadline:
                    raise TimeoutError('quiet interval exceeds batch deadline')
                client.quiet(quiet_deadline, trace)
            elif kind == 'poll_info':
                section = op.get('section', 'clients')
                field = op['field']
                expected = str(op['equals'])
                selected = list(op.get('record_fields', [])) + [field]
                if section == 'clients':
                    selected += ['blocked_clients', 'total_blocking_keys', 'total_blocking_keys_on_nokey']
                interval = milliseconds(op.get('interval_ms', 20), 'interval_ms')
                trace['attempts'] = []
                while True:
                    attempt = trace_for('poll_info', name)
                    trace['attempts'].append(attempt)
                    began = time.monotonic()
                    try:
                        client.send(enc(['INFO', section]), deadline, attempt)
                        raw, _, _ = client.reply(deadline, attempt)
                        fields = info_fields(raw)
                        attempt['fields'] = {key: fields.get(key) for key in dict.fromkeys(selected)}
                    finally:
                        attempt['elapsed_ms'] = round((time.monotonic() - began) * 1000, 3)
                        for key in ('sent', 'received', 'socket_received'):
                            trace[key] += attempt[key]
                        trace['frames'].extend(attempt['frames'])
                    if fields.get(field) == expected:
                        break
                    self.pause(interval, deadline)
            elif kind == 'rewrite':
                trace['aof_base'] = self.rewrite(client, deadline, trace)
            else:
                raise ValueError('unknown operation: ' + kind)

    def legacy(self, res):
        trace = trace_for('open', '__legacy__')
        client = self.open('__legacy__', self.batch.get('resp', 2), self.limit(), trace)
        for command in self.batch['commands']:
            trace = trace_for('command', '__legacy__')
            entry = {'cmd': command, 'reply': ''}
            res['replies'].append(entry)
            began = time.monotonic()
            try:
                if command == ['__RESTART__']:
                    deadline = self.limit(10000)
                    self.shutdown(deadline)
                    self.start(deadline)
                    client = self.open('__legacy__', self.batch.get('resp', 2), deadline, trace)
                elif command == ['__AOF__']:
                    files = self.aof()
                    entry['reply'] = files[-1]['bytes'] if files else ''
                elif command == ['__REWRITE__']:
                    entry['reply'] = self.rewrite(client, self.limit(30000), trace) or ''
                else:
                    client.send(enc(command), self.limit(), trace)
                    raw, _, _ = client.read(self.limit(), trace)
                    entry['reply'] = raw.decode('latin-1')
                    if self.batch.get('drain'):
                        while True:
                            try:
                                raw, _, _ = client.read(self.limit(300), trace)
                                entry['reply'] += raw.decode('latin-1')
                            except TimeoutError:
                                time_left(self.deadline)
                                break
            except Exception as error:
                trace['error'] = '%s: %s' % (type(error).__name__, error)
                trace['elapsed_ms'] = round((time.monotonic() - began) * 1000, 3)
                trace['buffered_bytes'] = bytes(client.buffer).decode('latin-1')
                entry['failure'] = trace
                raise
        if self.batch.get('rewrite_aof'):
            res['aof_base'] = self.rewrite(client, self.limit(30000), trace_for('rewrite', '__legacy__'))


def run(server, batch, port):
    res = {'name': batch.get('name', '<unnamed>'), 'resp': batch.get('resp', 2)}
    runner = None
    try:
        if ('commands' in batch) == ('operations' in batch):
            raise ValueError('batch requires exactly one of commands or operations')
        runner = Runner(server, batch, port)
        runner.start(runner.limit(10000))
        if 'commands' in batch:
            res['replies'] = []
            runner.legacy(res)
        else:
            for key, value in batch.items():
                if key not in ('name', 'resp', 'operations', 'timeout_ms', 'config', 'appendonly', 'rewrite_aof'):
                    res[key] = value
            res['operations'] = []
            for index, op in enumerate(batch['operations']):
                trace = trace_for(op.get('op'), op.get('client'))
                trace['index'] = index
                trace['request'] = op
                res['operations'].append(trace)
                began = time.monotonic()
                trace['started_ms'] = round((began - runner.started) * 1000, 3)
                try:
                    runner.operation(op, trace)
                except Exception as error:
                    trace['error'] = '%s: %s' % (type(error).__name__, error)
                    for name, client in runner.clients.items():
                        if client.buffer:
                            trace.setdefault('pending_buffers', {})[name] = bytes(client.buffer).decode('latin-1')
                    raise
                finally:
                    trace['elapsed_ms'] = round((time.monotonic() - began) * 1000, 3)
                    client = runner.clients.get(op.get('client'))
                    if client is not None and client.buffer:
                        trace['buffered_bytes'] = bytes(client.buffer).decode('latin-1')
    except Exception as error:
        res['error'] = '%s: %s' % (type(error).__name__, error)
    finally:
        if runner is not None:
            try:
                runner.cleanup()
            except Exception as error:
                res['cleanup_error'] = '%s: %s' % (type(error).__name__, error)
    return res


def main():
    out = []
    failed = False
    port = 7700
    try:
        server = sys.argv[1]
        for path in sorted(glob.glob('oracle/batches/*.json')):
            with open(path) as source:
                batches = json.load(source)
            if not isinstance(batches, list):
                raise ValueError('batch file must contain a list: ' + path)
            for batch in batches:
                port += 1
                result = run(server, batch, port)
                result['file'] = os.path.basename(path)
                out.append(result)
                print('== %s :: %s (RESP%s)' % (result['file'], result['name'], result['resp']), flush=True)
                for reply in result.get('replies', []):
                    print('  %r -> %r' % (reply['cmd'], reply['reply']), flush=True)
                if 'aof_base' in result:
                    print('  AOF base: %r' % result['aof_base'], flush=True)
                if 'error' in result or 'cleanup_error' in result:
                    failed = True
                    print('  ERROR: %s' % result.get('error', result.get('cleanup_error')), file=sys.stderr, flush=True)
    except Exception as error:
        failed = True
        out.append({'error': '%s: %s' % (type(error).__name__, error)})
        print(out[-1]['error'], file=sys.stderr)
    finally:
        with open('oracle-replies.json', 'w') as output:
            json.dump(out, output, indent=1)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
