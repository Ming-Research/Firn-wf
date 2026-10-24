#!/usr/bin/env python3
"""The comparison table of a reproduce.sh run.

    reproduce-table.py table OUT   write OUT/summary.md from the compare.csv of
                                   each OUT/results/probe-* cell and fail
                                   unless firn, its twin and every baseline
                                   line have a nonzero rate in the numbered
                                   passes of every cell
    reproduce-table.py selftest    check that on small result trees
"""
import csv
import glob
import os
import statistics
import sys
import tempfile

LINES = ['firn', 'firn-twin', 'reference', 'reference-io', 'valkey', 'valkey-io', 'dragonfly', 'garnet']


def table(out):
    """The markdown table of out's probe cells and what the cells lack."""
    cells = sorted(glob.glob(os.path.join(out, 'results', 'probe-*')))
    md = ['## Reproduced comparison, probe cells', '']
    missing = [] if cells else ['no probe cell']
    for cell in cells:
        label = os.path.basename(cell)
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
            return table(out)[1]

    full = [(line, passno, 100000 + 1000 * passno) for line in LINES for passno in (1, 2)]
    assert lacks(full) == [], 'a complete cell'
    with tempfile.TemporaryDirectory() as out:
        write(out, full)
        text = table(out)[0]
        assert '| set | 16 | 101500 | ' in text, text
    assert lacks([r for r in full if r[0] != 'dragonfly']) == ['probe-2: no rate for dragonfly'], 'a missing line'
    assert lacks([r if r[0] != 'garnet' else ('garnet', r[1], 0) for r in full]) == ['probe-2: no rate for garnet'], \
        'a zero rate'
    assert lacks([r if r[0] != 'valkey' else ('valkey', 'exact', r[2]) for r in full]) == \
        ['probe-2: no rate for valkey'], 'a line with only the exact pass'
    with tempfile.TemporaryDirectory() as out:
        assert table(out)[1] == ['no probe cell'], 'no cell'
        os.makedirs(os.path.join(out, 'results', 'probe-8'))
        assert table(out)[1] == ['probe-8: no compare.csv'], 'a cell without compare.csv'
    print('reproduce-table selftest: passed')


def main(argv):
    if argv[1:2] == ['selftest']:
        selftest()
    elif argv[1:2] == ['table'] and len(argv) == 3:
        text, missing = table(argv[2])
        with open(os.path.join(argv[2], 'summary.md'), 'w') as f:
            f.write(text + '\n')
        print(text)
        if missing:
            print('reproduce: the table is incomplete: ' + '; '.join(missing), file=sys.stderr)
            return 1
        print('reproduce: every probe cell has a rate for every line')
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
