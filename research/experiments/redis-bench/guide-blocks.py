#!/usr/bin/env python3
"""The commands of docs/benchmarking.md that CI runs literally.

Every ```bash block of the guide stands directly under a marker comment:

    <!-- reproduce: NAME -->        the block is a step CI runs, named NAME
    <!-- reproduce: skip: WHY -->   the block is not run, for the reason WHY

so that a command added to the guide cannot go unchecked. Usage:

    guide-blocks.py check DOC              fail unless every block is marked
    guide-blocks.py extract DOC [THROUGH [WITHOUT]]
                                           print the run steps in order, up to
                                           and including the step THROUGH ("-"
                                           for all), leaving out the step
                                           WITHOUT
    guide-blocks.py selftest               check the checks on small documents
"""
import re
import sys

MARKER = re.compile(r'^<!-- reproduce: (skip: \S.*|[a-z][a-z0-9-]*) -->$')


def steps(text):
    """The bash blocks of text as (name, body) pairs; name is None when skipped."""
    lines = text.split('\n')
    found = []
    marker = None
    i = 0
    while i < len(lines):
        line = lines[i]
        match = MARKER.match(line)
        if match:
            if marker:
                raise ValueError(f'line {i + 1}: a second marker before any block')
            marker = (match.group(1), i + 1)
        elif line.startswith('```bash'):
            if marker is None:
                raise ValueError(f'line {i + 1}: a bash block without a reproduce marker')
            start = i + 1
            i = start
            while i < len(lines) and lines[i] != '```':
                i += 1
            if i == len(lines):
                raise ValueError(f'line {start}: a bash block never closes')
            name = None if marker[0].startswith('skip') else marker[0]
            if name is not None and name in [n for n, _ in found]:
                raise ValueError(f'line {marker[1]}: the step {name} appears twice')
            found.append((name, '\n'.join(lines[start:i])))
            marker = None
        elif line.strip() and marker:
            raise ValueError(f'line {marker[1]}: the marker is not followed by a bash block')
        i += 1
    if marker:
        raise ValueError(f'line {marker[1]}: the marker is not followed by a bash block')
    if not [n for n, _ in found if n]:
        raise ValueError('the guide has no step to run')
    return found


def script(text, through=None, without=None):
    """The run steps of text as one Bash script, ending with the step through
    and leaving out the step without."""
    run = [(n, b) for n, b in steps(text) if n]
    names = [n for n, _ in run]
    for name in (through, without):
        if name and name not in names:
            raise ValueError(f'no step {name}; the steps are {" ".join(names)}')
    if through:
        run = run[:names.index(through) + 1]
    if without:
        run = [(n, b) for n, b in run if n != without]
    return ''.join(f'# step {n}\n{b}\n' for n, b in run)


def selftest():
    good = ('<!-- reproduce: one -->\n```bash\necho 1\n```\n\ntext\n\n'
            '<!-- reproduce: skip: slow -->\n```bash\nsleep 9\n```\n\n'
            '<!-- reproduce: two -->\n\n```bash\necho 2\n```\n```text\nout\n```\n')
    assert script(good) == '# step one\necho 1\n# step two\necho 2\n', script(good)
    assert script(good, 'one') == '# step one\necho 1\n'
    assert script(good, None, 'one') == '# step two\necho 2\n'
    assert script(good, 'two', 'two') == '# step one\necho 1\n'

    def refused(text, through=None, without=None):
        try:
            script(text, through, without)
        except ValueError:
            return True
        return False

    one = '<!-- reproduce: one -->\n```bash\ntrue\n```\n'
    assert not refused(one)
    assert refused(good, 'three'), 'a step the guide lacks'
    assert refused(good, None, 'three'), 'a left-out step the guide lacks'
    assert refused(one + '```bash\nrm -rf /\n```\n'), 'a block without a marker'
    assert refused(one + '<!-- reproduce: one -->\n```bash\ntrue\n```\n'), 'a repeated name'
    assert refused(one + '<!-- reproduce: two -->\nText.\n```bash\ntrue\n```\n'), 'text after a marker'
    assert refused(one + '<!-- reproduce: two -->\n'), 'a marker at the end'
    assert refused(one + '<!-- reproduce: a -->\n<!-- reproduce: b -->\n```bash\ntrue\n```\n'), 'two markers'
    assert refused(one + '<!-- reproduce: two -->\n```bash\ntrue\n'), 'an open block'
    assert refused('<!-- reproduce: skip: slow -->\n```bash\ntrue\n```\n'), 'a guide that runs nothing'
    assert refused(one + '<!-- reproduce: Two -->\n```bash\ntrue\n```\n'), 'a name outside [a-z0-9-]'
    print('guide-blocks selftest: passed')


def main(argv):
    try:
        if argv[1:2] == ['selftest']:
            selftest()
        elif argv[1:2] == ['check'] and len(argv) == 3:
            with open(argv[2], encoding='utf-8') as doc:
                found = steps(doc.read())
            print(f'{argv[2]}: {len([n for n, _ in found if n])} steps run, '
                  f'{len([n for n, _ in found if not n])} skipped')
        elif argv[1:2] == ['extract'] and len(argv) in (3, 4, 5):
            through, without = (argv[3:] + [None, None])[:2]
            with open(argv[2], encoding='utf-8') as doc:
                sys.stdout.write(script(doc.read(), None if through == '-' else through, without))
        else:
            print(__doc__, file=sys.stderr)
            return 2
    except ValueError as error:
        print(f'guide-blocks: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
