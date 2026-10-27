"""Extract the native UI API from the client's x2game.dll into crates/aa-lua/data/x2game.json.

    python -I x2game_api.py <x2game_unpacked.dll> <out.json>

Needs `pefile` (pip). Input: an unpacked x2game.dll of the 1.2 client (r208022; the shipped one is
packed; an unpacked copy is <client12-unpacked>/x2game_unpacked.dll). Addresses below are for that
build. Output:
  api_globals  names of the native API tables (ADDON, UI, X2Console ... X2Achievement), by API id
  events       UI event names by event id (FireUIEvent ids: 70 ENTERED_LOGIN, 477 ENTERED_WORLD_SELECT)
  bind_groups  per script-bind registration function, the [name, params] it registers
               (params None = could not be recovered)
"""
import collections, json, re, struct, sys
import pefile

path, out_path = sys.argv[1], sys.argv[2]
pe = pefile.PE(path, fast_load=True)
base = pe.OPTIONAL_HEADER.ImageBase
d = open(path, 'rb').read()
text = [s for s in pe.sections if s.Name.startswith(b'.text')][0]
lo, hi = text.PointerToRawData, text.PointerToRawData + text.SizeOfRawData


def off2va(o):
    return base + text.VirtualAddress + o - text.PointerToRawData


def va2off(v):
    r = v - base
    for s in pe.sections:
        if s.VirtualAddress <= r < s.VirtualAddress + s.SizeOfRawData and not s.Name.startswith(b'.text'):
            return s.PointerToRawData + r - s.VirtualAddress


def cstr(va, allow_empty=False):
    o = va2off(va)
    if o is None:
        return None
    e = d.find(b'\0', o, o + 300)
    s = d[o:e]
    if not s:
        return '' if allow_empty else None
    return s.decode() if all(32 <= c < 127 for c in s) else None


def func_start(o):
    i = o
    while i > lo:
        if d[i - 1] in (0xcc, 0x90) and d[i - 2] in (0xcc, 0x90) and d[i - 3] in (0xcc, 0x90):
            return i
        i -= 1
    return i


def u32(b):
    return struct.unpack('<I', b)[0]


# API table names: char* array indexed by API id (ADDON:ImportAPI checks id <= 0x46).
API_NAMES_VA = 0x39ac66a0
api = [cstr(u32(d[va2off(API_NAMES_VA + 4 * i):][:4])) for i in range(0x47)]

# Events: a static initialiser does `push <name>; mov ecx, <slot>; call <ctor>` per event; the
# slots are consecutive 4-byte objects and slot index = event id.
EVENT_CTOR = 0x39002bf0
PUSH, MOV_ECX, CALL = bytes([0x68]), bytes([0xb9]), bytes([0xe8])
slots = []
for m in re.finditer(re.escape(PUSH) + b'(....)' + re.escape(MOV_ECX) + b'(....)' + re.escape(CALL) + b'(....)', d[lo:hi], re.S):
    s_va, slot = u32(m.group(1)), u32(m.group(2))
    rel = struct.unpack('<i', m.group(3))[0]
    if off2va(lo + m.end()) + rel == EVENT_CTOR:
        slots.append((slot, cstr(s_va)))
slots.sort()
assert all(b[0] - a[0] == 4 for a, b in zip(slots, slots[1:])), 'event slots not contiguous'
events = [n for _, n in slots]

# Script binds: each registration fills a SUserFunctionDesc {name, params, ...} at [ebp+D] and calls
# IScriptTable vfunc +0x50:  mov r,[r+0x50] ; lea r2,[ebp+D] ; push r2 ; call r
ident = re.compile(r'^[A-Za-z_][A-Za-z0-9_]*$')
EBP_MODRM = {0x45: 0, 0x4d: 1, 0x55: 2, 0x5d: 3, 0x75: 6, 0x7d: 7}  # [ebp+d8] with reg eax..edi
LEA = bytes([0x8d])
MODRMS = b'[' + b''.join(re.escape(bytes([x])) for x in EBP_MODRM) + b']'
PUSHR = b'[' + re.escape(bytes([0x50])) + b'-' + re.escape(bytes([0x57])) + b']'
CALLR = re.escape(bytes([0xff])) + b'[' + re.escape(bytes([0xd0])) + b'-' + re.escape(bytes([0xd7])) + b']'
VF50 = re.escape(bytes([0x8b])) + b'[' + re.escape(bytes([0x40])) + b'-' + re.escape(bytes([0x7f])) + b']' + re.escape(bytes([0x50]))
MOV_EBP_IMM = re.escape(bytes([0xc7, 0x45]))
found = []
for m in re.finditer(re.escape(LEA) + b'(' + MODRMS + b')(.)' + PUSHR + CALLR, d[lo:hi], re.S):
    D = struct.unpack('b', m.group(2))[0]
    s = lo + m.start()
    back = d[max(lo, s - 220):s]
    if not re.search(VF50, back[-12:]):
        continue
    # this registration's block starts after the previous indirect call
    calls = [c.end() for c in re.finditer(CALLR, back)]
    cut = calls[-1] if calls else 0
    block = back[cut:]
    name = params = None
    for mm in re.finditer(MOV_EBP_IMM + re.escape(struct.pack('b', D)) + b'(....)', block, re.S):
        name = cstr(u32(mm.group(1)))
    for mm in re.finditer(MOV_EBP_IMM + re.escape(struct.pack('b', D + 4)) + b'(....)', block, re.S):
        params = cstr(u32(mm.group(1)), True)
    if params is None:
        # mov [ebp+D+4], reg: reg was loaded with `mov reg, imm32` earlier in the function
        for mm in re.finditer(re.escape(bytes([0x89])) + b'(' + MODRMS + b')' + re.escape(struct.pack('b', D + 4)), block, re.S):
            r = EBP_MODRM[mm.group(1)[0]]
            start = func_start(s)
            upto = s - len(back) + cut + mm.start()
            loads = list(re.finditer(re.escape(bytes([0xb8 + r])) + b'(....)', d[start:upto], re.S))
            if loads:
                params = cstr(u32(loads[-1].group(1)), True)
    if name and ident.match(name):
        found.append((off2va(func_start(s)), off2va(s), name, params))
groups = collections.OrderedDict()
for f, a, n, pa in sorted(found):
    groups.setdefault(hex(f), []).append([n, pa])

json.dump({
    'source': 'x2game.dll r208022 (unpacked); regenerate with crates/aa-lua/scripts/re/x2game_api.py',
    'api_globals': api,
    'events': events,
    'bind_groups': [{'register_fn': f, 'functions': l} for f, l in groups.items()],
}, open(out_path, 'w'), indent=1)
missing = sum(1 for f in found if f[3] is None)
print(f'{len(api)} API tables, {len(events)} events, {len(found)} functions in {len(groups)} bind groups, {missing} without params')
