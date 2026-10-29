#!/usr/bin/env python3
"""The host-dependent parts of a reproduce.sh run: the comparison table and,
on a host smaller than the reference 14900K, the CPU lists of a probe cell.

    reproduce-table.py table OUT [N...]   write OUT/summary.md from the
                                          compare.csv of each OUT/results/probe-N
                                          cell (the cells 2 and 8 by default)
                                          and fail unless firn, its twin and
                                          every baseline line have a nonzero
                                          rate in the numbered passes of
                                          every cell
    reproduce-table.py placement N        read `lscpu -p=CPU,CORE,SOCKET` on
                                          standard input and print the server
                                          and the client CPU lists of a cell
                                          with N server CPUs, apart by
                                          physical core
    reproduce-table.py selftest           check both on small inputs
"""
import csv
import os
import statistics
import sys
import tempfile

LINES = ['firn', 'firn-twin', 'reference', 'reference-io', 'valkey', 'valkey-io', 'dragonfly', 'garnet']


def table(out, counts=('2', '8')):
    """The markdown table of out's probe cells and what the cells lack."""
    md = ['## Reproduced comparison, probe cells', '']
    missing = []
    for count in counts:
        label = f'probe-{count}'
        cell = os.path.join(out, 'results', label)
        if not os.path.isdir(cell):
            missing.append(f'{label}: no result directory')
            continue
        try:
            with open(os.path.join(cell, 'compare.csv'), newline='') as f:
                rows = [r for r in csv.DictReader(f) if r['pass'] != 'exact']
        except FileNotFoundError:
            missing.append(f'{label}: no compare.csv')
            continue
        rate = {}
        for r in rows:
            try:
                value = float(r['rps'])
            except ValueError:
                continue
            if value > 0:
                rate.setdefault((r['test'], r['depth'], r['line']), []).append(value)
        seen = {key[2] for key in rate}
        missing += [f'{label}: no rate for {line}' for line in LINES if line not in seen]
        md += [f'### {label}: median requests per second over the numbered passes', '',
               '| test | depth | ' + ' | '.join(LINES) + ' |', '|---|---|' + '---|' * len(LINES)]
        for test, depth in sorted({(key[0], key[1]) for key in rate}):
            cols = [f'{statistics.median(rate[(test, depth, line)]):.0f}' if (test, depth, line) in rate else '-'
                    for line in LINES]
            md.append(f'| {test} | {depth} | ' + ' | '.join(cols) + ' |')
        md.append('')
    return '\n'.join(md), missing


def placement(lscpu, servers):
    """The server and client CPU lists of a cell with `servers` server CPUs
    on the host `lscpu -p=CPU,CORE,SOCKET` describes: the servers take the
    threads of the first physical cores in CPU order, the clients every thread
    of the cores after them, so that no client shares a core with a server."""
    cores = {}
    for line in lscpu.splitlines():
        if line.strip() and not line.startswith('#'):
            cpu, core, socket = (int(field) for field in line.split(',')[:3])
            cores.setdefault((socket, core), []).append(cpu)
    ordered = sorted(cores.values(), key=min)
    chosen = []
    taken = 0
    while taken < len(ordered) and len(chosen) < servers:
        chosen += sorted(ordered[taken])[:servers - len(chosen)]
        taken += 1
    if len(chosen) < servers:
        raise ValueError(f'the host has {len(chosen)} CPUs for {servers} server CPUs')
    clients = sorted(cpu for threads in ordered[taken:] for cpu in threads)
    if not clients:
        raise ValueError(f'no physical core is left for the clients of {servers} server CPUs')
    return ','.join(map(str, sorted(chosen))), ','.join(map(str, clients))


def selftest():
    def write(out, rows, cell='probe-2'):
        os.makedirs(os.path.join(out, 'results', cell))
        with open(os.path.join(out, 'results', cell, 'compare.csv'), 'w') as f:
            f.write('line,pass,cpus,test,depth,requests,rps,server_cpu_us_per_request,p50_ms,p99_ms,quantiles\n')
            for line, passno, rps in rows:
                f.write(f'{line},{passno},2,set,16,1000,{rps},1.0,0.1,0.2,upper_bound\n')

    def lacks(rows):
        with tempfile.TemporaryDirectory() as out:
            write(out, rows)
            return table(out, ('2',))[1]

    full = [(line, passno, 100000 + 1000 * passno) for line in LINES for passno in (1, 2)]
    assert lacks(full) == [], 'a complete cell'
    with tempfile.TemporaryDirectory() as out:
        write(out, full)
        text = table(out, ('2',))[0]
        assert '| set | 16 | 101500 | ' in text, text
        assert table(out)[1] == ['probe-8: no result directory'], 'a cell asked for and never run'
    assert lacks([r for r in full if r[0] != 'dragonfly']) == ['probe-2: no rate for dragonfly'], 'a missing line'
    assert lacks([r if r[0] != 'garnet' else ('garnet', r[1], 0) for r in full]) == ['probe-2: no rate for garnet'], \
        'a zero rate'
    assert lacks([r if r[0] != 'valkey' else ('valkey', 'exact', r[2]) for r in full]) == \
        ['probe-2: no rate for valkey'], 'a line with only the exact pass'
    with tempfile.TemporaryDirectory() as out:
        assert table(out)[1] == ['probe-2: no result directory', 'probe-8: no result directory'], 'no cell'
        os.makedirs(os.path.join(out, 'results', 'probe-8'))
        assert table(out, ('8',))[1] == ['probe-8: no compare.csv'], 'a cell without compare.csv'

    def cpus(*pairs):
        return ''.join(f'{cpu},{core},0\n' for cpu, core in pairs)

    header = '# The following is the parsable format\n# CPU,Core,Socket\n'
    # Two cores of two threads, the siblings numbered apart as on a cloud VM.
    smt = header + cpus((0, 0), (1, 1), (2, 0), (3, 1))
    assert placement(smt, 2) == ('0,2', '1,3'), 'both threads of a core serve, the other core serves clients'
    assert placement(smt, 1) == ('0', '1,3'), 'a server\'s sibling thread is no client'
    # Four cores of one thread.
    flat = header + cpus((0, 0), (1, 1), (2, 2), (3, 3))
    assert placement(flat, 2) == ('0,1', '2,3'), 'one thread per core'

    def refused(lscpu, servers):
        try:
            placement(lscpu, servers)
        except ValueError:
            return True
        return False

    assert refused(cpus((0, 0), (1, 0)), 2), 'a host without a core for the clients'
    assert refused(flat, 8), 'a host with fewer CPUs than servers'
    print('reproduce-table selftest: passed')


def main(argv):
    if argv[1:2] == ['selftest']:
        selftest()
    elif argv[1:2] == ['table'] and len(argv) >= 3:
        text, missing = table(argv[2], tuple(argv[3:]) or ('2', '8'))
        with open(os.path.join(argv[2], 'summary.md'), 'w') as f:
            f.write(text + '\n')
        print(text)
        if missing:
            print('reproduce: the table is incomplete: ' + '; '.join(missing), file=sys.stderr)
            return 1
        print('reproduce: every probe cell has a rate for every line')
    elif argv[1:2] == ['placement'] and len(argv) == 3 and argv[2].isdigit():
        try:
            print(*placement(sys.stdin.read(), int(argv[2])))
        except ValueError as error:
            print(f'reproduce: {error}', file=sys.stderr)
            return 1
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
