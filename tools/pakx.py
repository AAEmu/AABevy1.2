# Extract from 1.2 game_pak via the TSV index: python -I pakx.py <pak> <index.tsv> <outdir> <regex>
import sys, os, re
pak, tsv, outdir, rx = sys.argv[1:5]
rx = re.compile(rx, re.I)
f = open(pak, 'rb'); n = 0
for line in open(tsv, encoding='utf-8'):
    name, o, s = line.rstrip('\n').split('\t')
    if not rx.search(name): continue
    f.seek(int(o)); data = f.read(int(s))
    p = os.path.join(outdir, name); os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, 'wb').write(data); n += 1
print('extracted', n)
