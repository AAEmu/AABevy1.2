from pathlib import Path
import ctypes, hashlib, json, struct, subprocess

ROOT = Path(__file__).resolve().parents[4]
OUT = ROOT / 'data/extract/aa-world/ocean-foam-audit'

def unpack(src, want):
    ring = bytearray(b' ' * 4096)
    r, flags, p = 4096 - 18, 0, 0
    result = bytearray()
    while p < len(src) and len(result) < want:
        flags >>= 1
        if flags & 256 == 0:
            flags = src[p] | 0xff00
            p += 1
        if flags & 1:
            val = src[p]
            p += 1
            result.append(val)
            ring[r] = val
            r = (r + 1) & 4095
        else:
            a, z = src[p:p+2]
            p += 2
            i = a | ((z & 240) << 4)
            for j in range((z & 15) + 3):
                val = ring[(i+j) & 4095]
                result.append(val)
                ring[r] = val
                r = (r+1) & 4095
    assert want <= len(result) <= want + 17, (len(result), want)
    return bytes(result[:want])

d3d = ctypes.WinDLL('d3dcompiler_47.dll')
fn = d3d.D3DDisassemble
fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_uint, ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
fn.restype = ctypes.c_long

def disassemble(dxbc):
    blob = ctypes.c_void_p()
    buf = ctypes.create_string_buffer(dxbc)
    hr = fn(buf, len(dxbc), 0, None, ctypes.byref(blob))
    assert hr == 0, hr
    vt = ctypes.cast(blob, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
    ptr = ctypes.WINFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)(vt[3])
    length = ctypes.WINFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p)(vt[4])
    release = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(vt[2])
    text = ctypes.string_at(ptr(blob), length(blob)).decode(errors='replace')
    release(blob)
    return text

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for stage in ['ps', 'vs']:
        file = OUT / ('water@water' + stage + '.fxcb')
        kind = 'cgpshaders' if stage == 'ps' else 'cgvshaders'
        path = 'game/shaders/cache/d3d11/' + kind + '/water@water' + stage + '.fxcb'
        b = subprocess.check_output([str(ROOT/'tools/aa-pak.exe'), 'cat', path], cwd=ROOT)
        file.write_bytes(b)
        count, directory = struct.unpack_from('<I', b, 8)[0], struct.unpack_from('<I', b, 16)[0]
        seen, rows = set(), []
        for i in range(count):
            key, size, offset, other = struct.unpack_from('<4I', b, directory+i*16)
            size &= 0xffffff
            if offset in seen:
                continue
            seen.add(offset)
            want = struct.unpack_from('<I', b, offset)[0]
            data = unpack(b[offset+4:offset+size], want)
            pos = data.find(b'DXBC')
            if pos < 0:
                continue
            length = struct.unpack_from('<I', data, pos+24)[0]
            dxbc = data[pos:pos+length]
            text = disassemble(dxbc)
            name = 'native-' + stage + '-%08x' % key
            (OUT/(name+'.asm')).write_text(text)
            (OUT/(name+'.dxbc')).write_bytes(dxbc)
            (OUT/(name+'.prefix')).write_bytes(data[:pos])
            rows.append(dict(key=hex(key), offset=offset, size=length, prefix=data[:pos].hex(),
                             sha256=hashlib.sha256(dxbc).hexdigest(),
                             samples=[s for s in text.splitlines() if s.startswith('sample')]))
        (OUT/('cache-'+stage+'.json')).write_text(json.dumps(dict(source=str(file), source_sha256=hashlib.sha256(b).hexdigest(),
                                                               directory=directory, count=count, unique=len(rows), entries=rows), indent=2))
        print(stage, count, len(rows), 'unique original DXBC permutations')

if __name__ == '__main__':
    main()
