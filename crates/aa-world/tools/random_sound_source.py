"""Bounded RandomSoundVolume native/unchanged-ALB sampling oracle.

python crates/aa-world/tools/random_sound_source.py
Requires existing research Python pefile/capstone/unicorn and Lua5.1 lupa.
Only targeted original-pak reads and new ignored evidence writes. No game process/device.
FPCW is controlled, not asserted as the runtime game's mode. No global RNG sequence parity.
"""
from pathlib import Path
import hashlib, json, struct
import capstone, pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "data/extract/aa-world/random-sound-20261025"
OUT.mkdir(parents=True, exist_ok=True)
u = Uc(UC_ARCH_X86, UC_MODE_32)
pins, images = {}, {}
for name in ('cryscriptsystem', 'xlcommon'):
    raw = Path('<client12-unpacked>/ida/' + name + '.dll').read_bytes()
    pe = pefile.PE(data=raw)
    base = pe.OPTIONAL_HEADER.ImageBase
    image = pe.get_memory_mapped_image()
    u.mem_map(base, (len(image) + 4095) & ~4095)
    u.mem_write(base, image)
    pins[name] = hashlib.sha256(raw).hexdigest()
    assert pins[name] == {'cryscriptsystem': '75ad3b0e7b483b12a113fd6cb28bcab785259f28bca145cb3f28e0fa8e72f162', 'xlcommon': '0e0881aa837553d7e307a5c6f9d886f3e82a0d72d94b6d666448aedaa94e2203'}[name]
    images[name] = (base, image)
u.mem_map(0x70000000, 0x100000)
L, VALUES, STATE, STACK, STOP = 0x70001000, 0x70002000, 0x70003000, 0x700f0000, 0x700ff000

def put(a, *v): u.mem_write(a, struct.pack('<' + 'I' * len(v), *v))
def get(a): return struct.unpack('<I', u.mem_read(a, 4))[0]
def f32(v): return struct.unpack('<f', struct.pack('<f', v))[0]
def ret(v):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, v)
    u.reg_write(UC_X86_REG_EIP, get(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4)

def hook(_, a, size, unused):
    if a == 0x33017680: ret(STATE)  # Declared TLS state lookup endpoint; native generator executes.
    if a in (0x35548a30, 0x35547f50): raise AssertionError('unexpected Lua error')
u.hook_add(UC_HOOK_CODE, hook)
put(0x3556f430, 0x330171b0)  # Original CryRandomFloat import.
assert get(0x355773dc) == 0x35554a60
assert bytes(u.mem_read(get(0x355773d8), 7)) == b'random\0'
assert bytes(u.mem_read(0x35577464, 3)) == b'pi\0'
NATIVE_PI = struct.unpack('<f', u.mem_read(0x35577468, 4))[0]  # luaopen_math 35554C30 -> pushnumber -> setfield(pi).
assert NATIVE_PI == f32(3.141592653589793)

def undo(y, shift, mask, left):
    x = y
    for _ in range(32): x = (y ^ (((x << shift) if left else (x >> shift)) & mask)) & 0xffffffff
    return x
def untemper(y):
    for shift, mask, left in [(18, 0xffffffff, False), (15, 0xefc60000, True), (7, 0x9d2c5680, True), (11, 0xffffffff, False)]:
        y = undo(y, shift, mask, left)
    return y

def run(fn, args, word=0, cw=0x037f):
    u.reg_write(UC_X86_REG_FPCW, cw)
    u.reg_write(UC_X86_REG_FPSW, 0)
    u.reg_write(UC_X86_REG_MXCSR, 0x1f80)
    put(STATE, untemper(word))
    put(STATE + 0x9c4, 0)
    put(L + 8, VALUES + len(args) * 8, VALUES)
    for i, a in enumerate(args): u.mem_write(VALUES + i * 8, struct.pack('<fI', a, 3))
    put(STACK, STOP, L, 1)
    u.reg_write(UC_X86_REG_ESP, STACK)
    u.emu_start(fn, STOP, count=10000)
    assert u.reg_read(UC_X86_REG_EIP) == STOP
    if fn == 0x35548e20:
        x = u.reg_read(UC_X86_REG_EAX)
        return x if x < 0x80000000 else x - 0x100000000
    return struct.unpack('<f', u.mem_read(VALUES + len(args) * 8, 4))[0]

rows = []
for interval in [(10, 20), (15, 30), (0, NATIVE_PI)]:
    for word in (0, 1, 0x40000000, 0x80000000, 0xffffff00, 0xffffffff):
        unit = run(0x35554a60, [], word)
        result = run(0x35554a60, interval, word)
        rows.append(dict(interval=interval, tempered_word=hex(word), unit=unit, result=result))
conversions = []
for cw in (0x037f, 0x077f, 0x0b7f, 0x0f7f):
    conversions.append(dict(control_word=hex(cw), values=[(x, run(0x35548e20, [x], cw=cw)) for x in (0, 2.5, 3.5, f32(3.141592653589793), 10, 20, 15, 30)]))
assert rows[0]['result'] == 10 and rows[5]['result'] == 21
assert rows[6]['result'] == 15 and rows[11]['result'] == 31
assert rows[12]['result'] == 0 and rows[17]['result'] == 4
result = dict(pins=pins, random_registration='355773D8 -> 35554A60', pi_registration='luaopen_math35554C30 reads35577468 -> pushnumber35547180 -> setfield(pi)', native_pi=NATIVE_PI, endpoints=['TLS lookup returns controlled pretempered state; no global seed/TLS initialization parity', 'FPCW explicitly selected, MXCSR=0x1F80; no live game rounding-mode claim'], rows=rows, conversions=conversions)
(OUT / 'native-probe.json').write_text(json.dumps(result, indent=2) + '\n')
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
asm = []
for name, a, n in [('cryscriptsystem', 0x35548e20, 104), ('cryscriptsystem', 0x35546f10, 66), ('cryscriptsystem', 0x35554a60, 449), ('cryscriptsystem', 0x35554c30, 135), ('xlcommon', 0x330171b0, 17), ('xlcommon', 0x330173f0, 102)]:
    base, image = images[name]
    asm.append('FUNCTION ' + hex(a))
    asm += [f'{i.address:08X} {i.mnemonic:8s} {i.op_str}' for i in md.disasm(image[a-base:a-base+n], a)]
(OUT / 'native-probe.asm').write_text('\n'.join(asm) + '\n')
print("PASS: native integer conversion and unit RNG edge controls (including native unit1/max+1)")

from pathlib import Path
import math, subprocess, sys
from types import SimpleNamespace
native = SimpleNamespace(run=run, f32=f32)
import xml.etree.ElementTree as ET
sys.path.insert(0, str(ROOT / 'data/extract/aa-ui/research-lua-deps'))
from lupa.lua51 import LuaRuntime

pins = {}
def pak(name):
    b = subprocess.run([str(ROOT / 'tools/aa-pak.exe'), 'cat', name], check=True, capture_output=True).stdout
    pins[name] = hashlib.sha256(b).hexdigest()
    assert pins[name] == {
        'game/scriptsbin/entities/sound/randomsoundvolume.alb': '65487d29aa3d907332014802e4db7433b345442fc450c1bc05ca7171ec6f3bd5',
        'game/scripts/entities/sound/randomsoundvolume.lua': '0a5338532fcfeb48a5bf55730a34091a3ec73d41fae6d4ef9dbe06714adfb613',
        'game/worlds/main_world/cells/015_015/client/entities.xml': '1014e0cf3f31fffa8c88855fdafbe94a78e45ef82a403fd7f554e4687d677bd1',
    }[name]
    (OUT / Path(name).name).write_bytes(b)
    return b
b = pak('game/scriptsbin/entities/sound/randomsoundvolume.alb')
pak('game/scripts/entities/sound/randomsoundvolume.lua')
xml = pak('game/worlds/main_world/cells/015_015/client/entities.xml')
pos, records = 12, []
assert b[:12] == bytes.fromhex('1b4c75615100010404040400')
def take(n):
    global pos
    v = b[pos:pos+n]; pos += n; assert len(v) == n; return v
def integer(): return struct.unpack('<I', take(4))[0]
def string():
    n = integer(); return struct.pack('<Q', n) + take(n)
def function(key):
    out = string() + take(12); n = integer(); code = take(n*4); out += struct.pack('<I', n) + code
    n = integer(); out += struct.pack('<I', n)
    for _ in range(n):
        t = take(1); out += t
        if t == b'\x01': out += take(1)
        elif t == b'\x03': out += struct.pack('<d', struct.unpack('<f', take(4))[0])
        elif t == b'\x04': out += string()
        else: assert t == b'\x00'
    n = integer(); out += struct.pack('<I', n)
    for i in range(n): out += function(key + '/' + str(i))
    n = integer(); lines = take(n*4); out += struct.pack('<I', n) + lines
    n = integer(); out += struct.pack('<I', n)
    for _ in range(n): out += string() + take(8)
    n = integer(); out += struct.pack('<I', n)
    for _ in range(n): out += string()
    records.append(dict(key=key, instructions=len(code)//4, instruction_sha256=hashlib.sha256(code).hexdigest()))
    return out
chunk = bytes.fromhex('1b4c75615100010408040800') + function('main')
assert pos == len(b)

lua = LuaRuntime(encoding=None, unpack_returned_tuples=True)
g = lua.globals()
lua.eval(b'function(b) local f,e=loadstring(b); assert(f,e); f() end')(chunk)
g.math.pi = NATIVE_PI
rows, random_calls, events, words = [], [], [], []
def random(lo, hi):
    word = words.pop(0)
    value = native.run(0x35554a60, [lo, hi], word)
    random_calls.append(dict(interval=[lo, hi], word=hex(word), result=value))
    return value
def record(*args):
    vals = []
    for a in args:
        if isinstance(a, bytes): a = a.decode()
        elif hasattr(a, 'items'): a = {k.decode():v for k,v in a.items()}
        vals.append(a)
    events.append(vals)
g.math.random = random
g.record = record
lua.execute(b'''
SOUND_DEFAULT_3D=1; SOUND_START_PAUSED=2; SOUND_SEMANTIC_AMBIENCE_ONESHOT=4
bor=function(a,b) return a+b end
g_localActorId=7
g_Vectors={temp_v1={},v000={x=0,y=0,z=0}}
g_localActor={GetWorldPos=function(self,out) out.x=100; out.y=200; out.z=300; return out end}
Sound={PlayEx=function(name,pos,...) record('PlayEx',name,{x=pos.x,y=pos.y,z=pos.z}); return 99 end,
 SetSoundPaused=function(...) record('pause',...) end, StopSound=function(...) record('stop',...) end}
RandomSoundVolume.SetTimer=function(self,...) record('timer',...) end
RandomSoundVolume.KillTimer=function(self,...) record('kill',...) end
RandomSoundVolume.NetPresent=function(self,...) record('net',...) end
RandomSoundVolume.PlaySoundEventEx=function(self,name,flags,fade,pos,...) record('proxy',name,pos); return 98 end
RandomSoundVolume.GetName=function(self) return 'original-control' end
Log=function(...) record('log',...) end
''')
entity = g.RandomSoundVolume
original = ET.fromstring(xml)
placements = []
for el in original.iter('Entity'):
    if el.get('EntityClass') != 'RandomSoundVolume': continue
    props = el.find('Properties')
    owners = [x.get('Name') for x in original.iter('Entity') if any(y.get('EntityGuid') == el.get('EntityGuid') for y in x.findall('./Area/Entities/Entity'))]
    placements.append(dict(name=el.get('Name'), guid=el.get('EntityGuid'), properties=props.attrib, owners=owners))
assert len(placements) == 2 and all(p['owners'] == ['solz_sound_shape_sea06'] for p in placements)
for p in placements:
    props = entity.Properties
    for k,v in p['properties'].items():
        try: value = float(v)
        except ValueError: value = v.encode()
        props[k.encode()] = value
    entity.bEnabled = 1
    events.clear(); entity.Client.OnInit(entity)
    events.clear(); entity.Client.OnEnterArea(entity, lua.table_from({b'id':8}), 0, 1)
    assert events == []
    entity.Client.OnEnterArea(entity, lua.table_from({b'id':7}), 0, 1)
    assert events == [['timer', 0, float(p['properties']['MinWaitTime'])*1000]]
    rows.append(dict(name=p['name'], initial_enter=list(events)))
    for x in range(4):
        for y in range(4):
            events.clear(); random_calls.clear()
            words[:] = [0x80000000, (2*x+1)*0x20000000, (2*y+1)*0x20000000]
            entity.Client.OnTimer(entity, 0, 0)
            assert not words and len(random_calls) == 3
            play = next(v for v in events if v[0] == 'PlayEx')
            at = play[2]; radius = float(p['properties']['DiscRadius'])
            assert abs(at['x'] - (100+math.sin(x)*radius)) < 1e-9
            assert abs(at['y'] - (200+math.cos(y)*radius)) < 1e-9
            assert at['z'] == 300
            rows.append(dict(name=p['name'], x=x, y=y, random=list(random_calls), events=list(events)))
    events.clear(); entity.Client.OnLeaveArea(entity, lua.table_from({b'id':7}), 0, 1)
    assert events == [['stop',99],['kill',0],['kill',1]]
    rows.append(dict(name=p['name'], leave=list(events)))
events.clear(); random_calls.clear(); words[:] = [0]
entity.Properties.bRandomPosition = 0
entity.Play(entity)
assert len(random_calls) == 1 and next(x for x in events if x[0] == 'proxy')[2] == dict(x=0,y=0,z=0)
rows.append(dict(control='entity placement branch', random=list(random_calls), events=list(events)))
events.clear(); random_calls.clear(); entity.bEnabled = 0
entity.Play(entity)
assert not events and not random_calls
rows.append(dict(control='disabled Play', events=[]))
def current_rng(seed):
    x = seed | 1
    x ^= (x << 13) & 0xffffffffffffffff
    x ^= x >> 7
    x ^= (x << 17) & 0xffffffffffffffff
    return x, (x >> 40) / 2**24
cases = {}
for n in range(1, 10000):
    seed = n * 0x9e3779b97f4a7c15 & 0xffffffffffffffff
    state, a = current_rng(seed); state, b = current_rng(state); state, c = current_rng(state)
    pair = (int(b*4), int(c*4))
    if pair not in cases: cases[pair] = (seed, [a,b,c])
    if len(cases) == 16: break
assert len(cases) == 16
seed_rows = []
entity.bEnabled = 1; entity.Properties.bRandomPosition = 1
for pair, (seed, units) in sorted(cases.items()):
    random_calls.clear(); events.clear()
    words[:] = [int(v*2**32) for v in units]
    entity.Play(entity)
    timer = next(x[2] for x in events if x[0] == 'timer')
    target = next(x[2] for x in events if x[0] == 'PlayEx')
    wait10 = native.run(0x35554a60, [10, 20], int(units[0]*2**32))
    seed_rows.append(dict(seed=hex(seed), units=units, wait10=wait10, wait15=timer/1000, pair=pair, target=target, calls=list(random_calls)))
assert len({tuple(x['pair']) for x in seed_rows}) == 16
print('PASS: 16 retained existing-xorshift seed goldens execute original ALB and native math.random')
result = dict(pak_sha256=pins, bytecode=records, placements=placements, rows=rows, current_rng_seed_cases=seed_rows,
 endpoints=['Host Lua5.1 chunk layout widened only; every instruction unchanged', 'math.random executes original native math_random, integer check/conversion and CryRandomFloat, FPCW=0x037F, MXCSR=0x1F80; controlled pretempered TLS state, not global sequence', 'Host math.sin/cos double, local-actor position, timer dispatch, bit flags, sound API, net/name/log endpoints declared; no native trig/mixer/autoload or clock scheduling parity'], scope='Two authored bound Nuian consumers; all16 ordinary independent spatial pairs, initial local actor admission and delay, repeated timer Play, replacement and leave/disabled/entity-position controls')
(OUT / 'alb-probe.json').write_text(json.dumps(result,indent=2)+'\n')
print('PASS: unchanged original ALB; 2 authored Nuian placements, 32 spatial Play calls, local actor/first delay/replacement/leave/entity/disabled controls')
