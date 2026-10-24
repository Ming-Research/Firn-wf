#!/usr/bin/env python3
"""Temporary check for firn-cite-wf-claims (deleted before the merge).

Checks docs/safety.md against a Whitefoot clone: the links to the claims page
resolve to headings the page has, every link reference is defined and used,
and the comparison of the page's rules between the pin and the page's commit
says what the document says. Then runs the same check on broken copies of the
document, each of which must fail.
"""
import difflib
import re
import subprocess
import sys

PIN = 'af7665be672d29912d878dd5da8c3932bb6f296c'
PAGE = '4b58ba6dc219a19a5e8c98778faf3ce5b8b20982'
WF = sys.argv[1]
DOC = 'docs/safety.md'


def show(rev, path):
    out = subprocess.run(['git', '-C', WF, 'show', f'{rev}:{path}'],
                         check=True, capture_output=True, text=True)
    return out.stdout


def rules(spec):
    found, cur = {}, None
    for line in spec.splitlines(keepends=True):
        m = re.match(r'^\[([A-Z]+-\d+)\]\s', line)
        if m:
            cur = m.group(1)
            found[cur] = [line]
        elif line.startswith('#'):
            cur = None
        elif cur:
            found[cur].append(line)
    return {k: ''.join(v) for k, v in found.items()}


def slug(heading):
    text = re.sub(r'[^\w\s-]', '', heading.lower())
    return re.sub(r'\s', '-', text.strip())


page = show(PAGE, 'docs/safety-claims.md')
anchors = {slug(h) for h in re.findall(r'^#+ (.+)$', page, re.M)}
at_pin, at_page = rules(show(PIN, 'spec/kernel-spec.md')), \
    rules(show(PAGE, 'spec/kernel-spec.md'))
cited = sorted(set(re.findall(r'\b([A-Z]{2,6}-\d+)\b', page)))
missing = [r for r in cited if r not in at_pin]
differ = [r for r in cited if r in at_pin and at_pin[r] != at_page.get(r)]
quotes = re.findall(r'^\| [^|]+ \| “([^”]+)” \| `([A-Z]+-\d+)` \|$', page, re.M)
print(f'page cites {len(cited)} rules, {len(missing)} missing at the pin, '
      f'{len(differ)} differ: {differ}; {len(quotes)} quotes')
for rule in differ:
    print(f'--- {rule}: pin -> page commit')
    old, new = at_pin[rule].splitlines(), at_page[rule].splitlines()
    for line in difflib.unified_diff(old, new, lineterm='', n=0):
        if not line.startswith(('---', '+++', '@@')):
            print(line[:600])
print('--- end of the differing rules')


def check(doc):
    errors = []
    flat = ' '.join(doc.split())
    defs = dict(re.findall(r'^\[([^\]]+)\]: (\S+)$', doc, re.M))
    body = re.sub(r'^\[[^\]]+\]:.*$', '', doc, flags=re.M)
    used = set(re.findall(r'\]\[([^\]]+)\]', body))
    errors += [f'unused reference {d}' for d in defs if d not in used]
    errors += [f'undefined reference {u}' for u in used if u not in defs]
    base = f'https://github.com/Ming-Research/Whitefoot/blob/{PAGE}/'
    for name, url in defs.items():
        if '/docs/safety-claims.md' not in url:
            continue
        path, _, frag = url.partition('#')
        if path != base + 'docs/safety-claims.md':
            errors.append(f'{name}: not the page at {PAGE[:7]}')
        if frag and frag not in anchors:
            errors.append(f'{name}: the page has no heading #{frag}')
    if f'`{PAGE[:7]}`' not in doc:
        errors.append(f'the document does not name {PAGE[:7]}')
    m = re.search(r'the (\d+) rules the page cites all exist at the pin; '
                  r'(\d+) read the same there, and (.+?) differ only by',
                  flat)
    if not m:
        errors.append('the comparison sentence is missing')
    else:
        stated = sorted(re.findall(r'[A-Z]+-\d+', m.group(3)))
        if int(m.group(1)) != len(cited) or missing:
            errors.append(f'{m.group(1)} rules stated, {len(cited)} cited')
        if int(m.group(2)) != len(cited) - len(differ):
            errors.append(f'{m.group(2)} same stated, '
                          f'{len(cited) - len(differ)} found')
        if stated != differ:
            errors.append(f'differing rules stated {stated}, found {differ}')
    for text, rule in quotes:
        if ' '.join(text.split()) not in ' '.join(at_pin[rule].split()):
            errors.append(f'quote of {rule} is not verbatim at the pin')
    return errors


doc = open(DOC, encoding='utf-8').read()
errors = check(doc)
for e in errors:
    print('FAIL', e)
if errors:
    sys.exit(1)
print('the document passes')

broken = {
    'a heading that is not on the page': ('#no-data-races', '#no-data-race'),
    'a rule count that is not the page\'s': ('the 35 rules', 'the 36 rules'),
    'a differing rule left out of the list': ('WAIT-2 differ', 'differ'),
    'the page linked at a commit that has no page': (PAGE, PIN),
}
for what, (old, new) in broken.items():
    assert old in doc, old
    result = check(doc.replace(old, new))
    print(f'{what}: {"caught" if result else "NOT CAUGHT"} {result[:2]}')
    if not result:
        sys.exit(1)
print('every broken copy failed the check')
