# List the 1.2 game_pak file table: python -I paklist.py <pak> <out.tsv>   (name \t offset \t size)
import sys, struct
from Crypto.Cipher import AES
KEY = bytes([0x32,0x1F,0x2A,0xEE,0xAA,0x58,0x4A,0xB4,0x9A,0x6C,0x9E,0x09,0xD5,0x9E,0x9C,0x6F])
def dec(b): return AES.new(KEY, AES.MODE_CBC, iv=bytes(16)).decrypt(b)
def nm(d, at): return d[at:at+0x108].split(b'\0')[0].decode('latin1')
f = open(sys.argv[1], 'rb'); f.seek(0, 2); ln = f.tell()
f.seek(ln - 0x200); h = dec(f.read(0x200))
if h[0:4] == b'WIBO': t, fc, xc = 'A', *struct.unpack_from('<II', h, 8)
elif h[8:12] == b'IDEJ': t, fc, xc = 'B', struct.unpack_from('<I', h, 12)[0], struct.unpack_from('<I', h, 0)[0]
elif h[0:4] == b'ZERO': t, fc, xc = 'F', *struct.unpack_from('<II', h, 8)
else: raise SystemExit('bad header')
tot = (fc + xc) * 0x150
off = ln - 0x200 - tot; off -= off % 0x200
f.seek(off); fat = f.read(tot)
out = open(sys.argv[2], 'w', encoding='utf-8', newline='\n')
n = 0
for i in range(fc + xc):
    d = dec(fat[i*0x150:(i+1)*0x150])
    if t == 'A': name, o, s = (nm(d, 0), *struct.unpack_from('<qq', d, 0x108))
    elif t == 'B': name, s, o = nm(d, 0x20), struct.unpack_from('<q', d, 0x130)[0], struct.unpack_from('<q', d, 0x18)[0]
    else: name, o, s = (nm(d, 8), *struct.unpack_from('<qq', d, 0x110))
    isfile = (i < fc) if t in 'AF' else (i >= xc)
    if isfile: out.write(f'{name}\t{o}\t{s}\n'); n += 1
print('type', t, 'files', n)
