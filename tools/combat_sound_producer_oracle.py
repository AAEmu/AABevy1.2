"""Read-only isolated execution of original 1.2 combat-sound instructions.

No game process, device, build, original binary copy, or source edits. SQLite rows
populate the native record fields used here. Hooks replace only item lookup,
actor model accessor, DB enumeration and external Sound/serializer methods.
Native holdable/model range lookups, material choice, hit whitelist, parameter
branches and setter/Play ordering execute from the SHA-pinned original DLL.
These are instruction-level cases, not a live-client capture or CPU benchmark.
"""
from pathlib import Path
import hashlib
import json
import sqlite3
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ECX,
    UC_X86_REG_EDI, UC_X86_REG_EIP, UC_X86_REG_ESI, UC_X86_REG_ESP,
)

HERE = Path(__file__).resolve().parents[1] / 'data/extract/aa-client/combat-sound-producer'
HERE.mkdir(parents=True, exist_ok=True)
ORIGINAL = Path('D:/aa-re/client12/x2game_unpacked.dll')
SHA = '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
assert hashlib.sha256(ORIGINAL.read_bytes()).hexdigest() == SHA
pe = pefile.PE(str(ORIGINAL))
mapped = pe.get_memory_mapped_image()
BASE = pe.OPTIONAL_HEADER.ImageBase
assert BASE == 0x38ff0000
uc = Uc(UC_ARCH_X86, UC_MODE_32)
uc.mem_map(BASE, (len(mapped) + 4095) & ~4095)
uc.mem_write(BASE, mapped)
uc.mem_map(0x10000000, 0x200000)
DB, CTX, UNIT, TARGET = 0x10000000, 0x10020000, 0x10021000, 0x10023000
SOUND, VT, PARAM, PVT = 0x10025000, 0x10025100, 0x10025300, 0x10025400
ACTOR, AVT, MODELID = 0x10025600, 0x10025700, 0x10025800
ITEM, MAPROWS, IDS = 0x10026000, 0x10030000, 0x10033000
HOLROWS, HOLPTR, HOLRANGE = 0x10040000, 0x10043000, 0x10044000
MODROW, MODPTR, MODRANGE = 0x10045000, 0x10045100, 0x10045200
STREAM, SVT = 0x10046000, 0x10046100
FRAME, STOP = 0x100e0000, 0x101ff000
SET, FADE, GETPARAM, VOLUME, LOCAL, POSITION, PLAY, GETMODEL = range(0x101f0000, 0x101f0080, 0x10)
READ8, READ16 = 0x101f0100, 0x101f0110
ACTORTYPE, SKILL = 0x101f0120, 0x10048000
con = sqlite3.connect('file:D:/aaemu-live/rs-game/Data/compact.sqlite3?mode=ro', uri=True)
holdables = {r[0]: r[1:] for r in con.execute('select id,slot_type_id,sound_material_id from holdables')}
items = dict(con.execute('select item_id,holdable_id from item_weapons'))
maps = con.execute('select id,fx_group_id,sound_id,source_sound_material_id,target_sound_material_id from combat_sounds order by id').fetchall()

def w32(a, v): uc.mem_write(a, struct.pack('<I', v & 0xffffffff))
def r32(a): return struct.unpack('<I', uc.mem_read(a, 4))[0]
def wf(a, v): uc.mem_write(a, struct.pack('<f', v))
def rf(a): return struct.unpack('<f', uc.mem_read(a, 4))[0]
def cstr(a):
    raw = bytes(uc.mem_read(a, 128))
    return raw.split(b'\0', 1)[0].decode()
def ret(value=0, popped=0):
    sp = uc.reg_read(UC_X86_REG_ESP)
    uc.reg_write(UC_X86_REG_EAX, value)
    uc.reg_write(UC_X86_REG_EIP, r32(sp))
    uc.reg_write(UC_X86_REG_ESP, sp + 4 + popped)
def arg(n=0): return r32(uc.reg_read(UC_X86_REG_ESP) + 4 + n*4)

w32(0x39acb0e8, DB)
for hid, (slot, material) in holdables.items():
    p = HOLROWS + hid*0x90
    w32(p, hid); w32(p+0x74, slot); w32(p+0x78, material)
    w32(HOLPTR+hid*4, p)
w32(HOLRANGE, 0); w32(HOLRANGE+4, max(holdables)); w32(HOLRANGE+8, HOLPTR)
w32(DB+0x2ca8, HOLRANGE); w32(DB+0x2cac, HOLRANGE+12)
mid, material = con.execute('select id,sound_material_id from models where id=1035').fetchone()
w32(MODROW, mid); w32(MODROW+0x28, material)
w32(MODPTR, MODROW)
w32(MODRANGE, mid); w32(MODRANGE+4, mid); w32(MODRANGE+8, MODPTR)
w32(DB+0x2c40, MODRANGE); w32(DB+0x2c44, MODRANGE+12)
w32(MODELID, mid); w32(ACTOR, AVT); w32(AVT+8, GETMODEL)
w32(AVT+4, ACTORTYPE)
w32(0x3a5ad970, 0)  # empty target chest item at TARGET+0x638
for n, (_, fx, sound, source, target) in enumerate(maps):
    uc.mem_write(MAPROWS+n*16, struct.pack('<4I', fx or 0, sound or 0, source or 0, target or 0))
w32(SOUND, VT); w32(PARAM, PVT)
for offset, fn in [(0x64,SET),(0x30,FADE),(0xb8,GETPARAM),(0x3c,POSITION),(0x20,PLAY)]:w32(VT+offset, fn)
w32(PVT+0xc0, VOLUME); w32(PVT+0x11c, LOCAL)
w32(STREAM, SVT); w32(SVT+0x44, READ8); w32(SVT+0x40, READ16)
w32(TARGET+0x1000, 5569); w32(TARGET+0x10c0, 6152)
trace, wire, reads = [], b'', []
actor_type = 1
units = {111: UNIT, 222: TARGET}

def hook(emu, address, size, _):
    global wire
    if address == STOP:
        emu.emu_stop()
    elif address == 0x396a6d40:
        hid = items.get(arg())
        if hid is None:ret(0)
        else:w32(ITEM+0x20, hid); ret(ITEM)
    elif address == 0x390b3750:ret(ACTOR)
    elif address == 0x39360020:ret(units.get(arg(),0))
    elif address == ACTORTYPE:ret(actor_type)
    elif address == GETMODEL:ret(MODELID, 4)
    elif address == 0x396e2520:
        out = arg()
        uc.mem_write(out, struct.pack('<3I', IDS, IDS+4*len(maps), MAPROWS));ret(out)
    elif address == SET:
        trace.append({'method':'parameter','name':cstr(arg()),'value':rf(uc.reg_read(UC_X86_REG_ESP)+8),'immediate':arg(2)})
        ret(1,12)
    elif address == GETPARAM:ret(PARAM)
    elif address == FADE:trace.append({'method':'sound+0x30','float':rf(uc.reg_read(UC_X86_REG_ESP)+4),'second':arg(1)});ret(0,8)
    elif address == VOLUME:trace.append({'method':'volume','value':rf(uc.reg_read(UC_X86_REG_ESP)+4)});ret(0,4)
    elif address == LOCAL:trace.append({'method':'local','value':arg()});ret(0,4)
    elif address == POSITION:trace.append({'method':'position','xyz':list(struct.unpack('<3f',uc.mem_read(arg(),12)))});ret(0,4)
    elif address == PLAY:trace.append({'method':'play','args':[rf(uc.reg_read(UC_X86_REG_ESP)+4),arg(1),arg(2),arg(3)]});ret(1,16)
    elif address in (READ8, READ16):
        n=1 if address==READ8 else 2
        name=cstr(arg());dest=arg(1)
        assert len(wire)>=n
        uc.mem_write(dest,wire[:n]);wire=wire[n:];reads.append(name);ret(1,12)

uc.hook_add(UC_HOOK_CODE,hook)
def run(start, end=STOP):
    uc.emu_start(start,end,count=10000)
    assert uc.reg_read(UC_X86_REG_EIP)==end,hex(uc.reg_read(UC_X86_REG_EIP))

def select(hit, hol):
    w32(CTX+0x48,hit);w32(CTX+0x118,hol)
    sp=FRAME-0x1000;uc.mem_write(sp,struct.pack('<4I',STOP,CTX,UNIT,TARGET))
    uc.reg_write(UC_X86_REG_ESP,sp);uc.reg_write(UC_X86_REG_ECX,0x10047000)
    run(0x393913c0)
    p=uc.reg_read(UC_X86_REG_EAX)
    if not p:return None
    n=(p-MAPROWS)//16
    assert p==MAPROWS+n*16
    return {'combat_sound_id':maps[n][0], 'sound_id':maps[n][2], 'source_material':maps[n][3], 'target_material':maps[n][4]}

def params(hit, weapon, pierce):
    trace.clear();w32(CTX+0x48,hit);w32(CTX+0x64,pierce);w32(UNIT+0x1000,weapon)
    w32(FRAME+0xc,UNIT);wf(FRAME+8,0.25);w32(FRAME-0x10,0)
    uc.mem_write(FRAME-0x30,struct.pack('<3f',1,2,3))
    for reg,value in [(UC_X86_REG_ESP,FRAME-0x100),(UC_X86_REG_EBP,FRAME),(UC_X86_REG_ESI,SOUND),(UC_X86_REG_EDI,CTX),(UC_X86_REG_EBX,ITEM)]:uc.reg_write(reg,value)
    run(0x393f2495,0x393f265f)
    p={x['name']:x['value'] for x in trace if x['method']=='parameter'}
    assert list(p)==['pierce','miss','critical','twohand']
    assert all(x['immediate']==1 for x in trace if x['method']=='parameter')
    assert p['critical']==float(hit in (3,11))
    assert p['miss']==float(hit in (4,10,14))
    assert p['twohand']==float(holdables.get(items.get(weapon),[None])[0]==16)
    assert p['pierce']==(1.0 if pierce==1 else struct.unpack('<f',struct.pack('<f',0.3))[0] if pierce==2 else 0.0)
    assert trace[-1]=={'method':'play','args':[1.0,1,1,0]}
    assert next(x['value'] for x in trace if x['method']=='volume')==(1.0 if hit in (6,7,17,20) else 0.25)
    return list(trace)

cases=[]
for hit in range(21):
    for weapon in (5660,5569,6102,0):
        for pierce in (0,1,2,15):
            events=params(hit,weapon,pierce)
            selected=select(hit,items.get(weapon,0))
            assert bool(selected)==(hit in (1,3,4,6,7,9,10,11,17,20))
            cases.append({'hit_type':hit,'mainhand_item':weapon,'context_holdable':items.get(weapon,0),'high_nibble':pierce,'selection':selected,'events':events})

wire_cases=[]
for hit in (1,3,4,11,15):
    for high in (0,1,2,15):
        packed=high<<12|0x120|hit;wire=struct.pack('<BHB',3,packed,1);reads.clear()
        w32(FRAME+8,STREAM)
        for reg,value in [(UC_X86_REG_ESP,FRAME-0x100),(UC_X86_REG_EBP,FRAME),(UC_X86_REG_ESI,CTX),(UC_X86_REG_EBX,0),(UC_X86_REG_ECX,STREAM)]:uc.reg_write(reg,value)
        run(0x397c88ae,0x397c894c)
        assert not wire and reads==['hol','de','flag']
        decoded={'holdable':r32(CTX+0x118),'hit_type':r32(CTX+0x48),'bits5_7':r32(CTX+0x44),'bits8_11':r32(CTX+0x4c),'high_nibble':r32(CTX+0x64)}
        assert decoded=={'holdable':3,'hit_type':hit,'bits5_7':1,'bits8_11':1,'high_nibble':high}
        wire_cases.append({'packed':hex(packed),'decoded':decoded})

timing_cases=[]
for name, hit, radius, src, dst, kind, expected in [
    ('single_target_actor1_waits_for_hit',3,0,111,222,1,False),
    ('normal_hit_also_waits',1,0,111,222,1,False),
    ('miss_also_waits',4,0,111,222,1,False),
    ('area_radius_is_immediate',3,5,111,222,1,True),
    ('invalid_hit_is_immediate_but_selector_rejects',0,0,111,222,1,True),
    ('self_is_immediate',3,0,111,111,1,True),
    ('missing_source_is_immediate',3,0,333,222,1,True),
    ('missing_target_is_immediate',3,0,111,333,1,True),
    ('other_actor_type_is_immediate',3,0,111,222,2,True),
]:
    actor_type=kind
    w32(CTX+0x48,hit);w32(SKILL+0x188,radius)
    w32(CTX+0x18,0);w32(CTX+0x20,src);w32(CTX+0x3c,dst)
    sp=FRAME-0x1000;uc.mem_write(sp,struct.pack('<3I',STOP,CTX,SKILL))
    uc.reg_write(UC_X86_REG_ESP,sp)
    run(0x39391950)
    actual=bool(uc.reg_read(UC_X86_REG_EAX)&255)
    assert actual==expected,name
    timing_cases.append({'case':name,'hit_type':hit,'skills_target_area_radius':radius,'source':src,'target':dst,'actor_virtual4_type':kind,'immediate':actual})

result={'original':str(ORIGINAL),'sha256':SHA,'case_count':len(cases),'wire_case_count':len(wire_cases),'timing_case_count':len(timing_cases),'limitations':__doc__,'cases':cases,'wire_cases':wire_cases,'timing_cases':timing_cases}
(HERE/'native_producer_cases.json').write_text(json.dumps(result,indent=2)+'\n')
selected=[c for c in cases if c['hit_type'] in (1,3,4,6,7,11,15) and c['mainhand_item']==5660 and c['high_nibble']==0]
print(json.dumps({'case_count':len(cases),'wire_case_count':len(wire_cases),'timing_case_count':len(timing_cases),'examples':[{'hit':c['hit_type'],'selection':c['selection'],'parameters':c['events'][:4]} for c in selected]},indent=2))
