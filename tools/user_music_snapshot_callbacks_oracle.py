"""Bounded native snapshot/body, hydration/replay/binding notification audit.

Original caller bodies execute with listed external/presentation sinks. No live
process, server, device or production changes. Outputs are retained under tools/evidence/user_music_actions.
"""
from pathlib import Path
import hashlib
import json
import struct
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from user_music_lifecycle_oracle import Native, WORLD, RUNTIME_DATA, RUNTIME_SHA, SHA
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_ECX, UC_X86_REG_EDI, UC_X86_REG_EIP, UC_X86_REG_ESP

OUT = Path(__file__).parent / "evidence" / "user_music_actions"
n = Native()
SNAP, BUFFS, SELECTED, BAR, VECTOR = 0x10090000, 0x10094000, 0x100A9000, 0x100B0000, 0x100C0000
GUI, GVT, RENDER, RVT, CONTROL, POS, CELL, VOID, TREE = [0x100F0000+i*0x1000 for i in range(9)]
OBJ, CHAR = 47, 11
actor = [0]
registry = [False]
log, ui, sink_calls, gate_frames = [], [], [], []
runtime = RUNTIME_DATA.read_bytes()
assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
for at in [0x39D1FB9C, 0x39AC62BC, 0x3A1744D0, 0x3A1744D4, 0x3A1744E0,
           0x3A1744E8, 0x39B02334, 0x39D21164, 0x39D2116C, 0x39AC65BC, 0x39AC65C0,
           0x3A1699F4, 0x3A169968, 0x3A169A0C]:
    n.put(at, struct.unpack_from('<I', runtime, at-0x39AC3000)[0])
assert n.get(0x39B02334) == n.get(0x3A1744E8) == 0

def sink(address, pop=0, value=0):
    def callback():
        sink_calls.append(hex(address)); n.ret(value, pop)
    n.hooks[address] = callback

def endpoint(address, pop=0, value=0):
    sink(address, pop, value); return address

n.put(0x39B02294, WORLD)
n.put(WORLD+0x10C, 0)
n.put(WORLD+0x40, SELECTED)
n.put(SELECTED+0x18, CHAR)
n.put(WORLD+4, GUI)
n.put(GUI, GVT)
n.put(GVT+0x1EC, endpoint(0x30000700, value=GUI))
n.put(GVT+0x50, endpoint(0x30000710, value=GUI))
n.put(GVT+0x34, endpoint(0x30000720))
n.put(GVT+0x1C, endpoint(0x30000730, pop=4))
n.put(n.get(0x3A5BB1D8)+4, GUI)
n.put(RENDER, RVT)
n.put(RVT+0x208, endpoint(0x30000740, value=CONTROL))
n.put(RVT+4, endpoint(0x30000750))
n.put(RVT+0x1C4, endpoint(0x30000760, pop=16))
n.put(RVT+0x258, endpoint(0x30000770, pop=4))
n.put(RVT+0x264, endpoint(0x30000780))
n.put(RVT+0x1E0, endpoint(0x30000790))
n.put(RVT+0x1B8, endpoint(0x300007A0))
def ground():
    n.u.mem_write(n.arg(0), bytes(12)); n.ret(pop=16)
n.hooks[0x300007B0] = ground
n.put(RVT+0xB0, 0x300007B0)
n.put(CONTROL+8, OBJ)
n.put(CELL, OBJ)
n.put(TREE, TREE, TREE, TREE)
n.u.mem_write(TREE+0x11, b'\1')
n.put(0x39AFFA38, BAR)
n.put(0x3A1744F0, VECTOR, VECTOR, VECTOR+0x1000)
n.put(0x3A174880, 1)
for at in [0x3A16E8A0, 0x3A176140, 0x3A16CFF8]: n.put(at, 1)
n.put(0x39D1F970, 1)

for address, pop, value in [(0x390115A0,16,0), (0x390116D0,0,0), (0x398A1F90,0,0),
                          (0x398A0170,8,0), (0x39899D10,8,0), (0x39695A70,0,0),
                          (0x390B95E0,16,0), (0x3936A630,0,0), (0x39362770,8,0),
                          (0x3917E0A0,4,0), (0x39360920,0,0), (0x390B4730,0,0),
                          (0x3935C270,4,0), (0x3936A430,0,0), (0x39362450,8,VOID),
                          (0x3935D130,8,0), (0x3900C1E0,4,0), (0x39387E00,4,0),
                          (0x39387C30,0,0), (0x390281C0,0,0), (0x3936BAA0,4,0),
                          (0x3916C130,0,0), (0x391645B0,0,0), (0x390B3850,0,POS),
                          (0x390B3750,0,RENDER), (0x390B39C0,4,CELL), (0x392F8030,0,0),
                          (0x39386570,0,VOID), (0x39386030,0,0), (0x39202DB0,8,0),
                          (0x39204500,0,0), (0x390B97D0,0,0), (0x393562E0,0,0)]:
    sink(address, pop, value)
n.put(0x38FF6F34, endpoint(0x300007C0))
n.put(0x38FF6F30, endpoint(0x300007D0))
def coords():
    n.u.mem_write(n.arg(0), bytes(24)); n.ret(n.arg(0), 4)
n.hooks[0x3961DE90] = coords
def relative_coords():
    n.u.mem_write(n.arg(0), bytes(24)); n.ret(n.arg(0), 8)
n.hooks[0x3961DE30] = relative_coords
def marker():
    n.put(n.arg(1), 0, 0); n.ret(n.arg(1), 8)
n.hooks[0x3983C4B0] = marker
def parent_id():
    n.put(n.arg(0), n.get(0x3A169A0C)); n.ret(n.arg(0), 4)
n.hooks[0x390B4990] = parent_id
def listener():
    n.put(n.arg(0), 0); n.ret(n.arg(0))
n.hooks[0x39028240] = listener
def ctor():
    actor[0] = n.u.reg_read(UC_X86_REG_ECX)
    n.put(actor[0], 0x399D0918)
    n.put(actor[0]+8, n.arg(0))
    n.put(actor[0]+0x3A7C, 100)
    n.put(actor[0]+0x3BE8, RENDER)
    n.put(actor[0]+0x5F98, TREE)
    n.put(actor[0]+0x3E80, 0x399D1990)
    log.append({'call':'39367d30 constructor sink', 'actor': actor[0], 'bound': n.get(WORLD+0x10C)})
    n.ret(actor[0], 4)
n.hooks[0x39367D30] = ctor
def lookup():
    log.append({'call':'39360020 lookup', 'obj':n.arg(0), 'exists':registry[0]})
    n.ret(actor[0] if registry[0] and n.arg(0)==OBJ else 0)
n.hooks[0x39360020] = lookup
def registered():
    registry[0] = True
n.hooks[0x3936B60A] = registered
n.hooks[0x39388440] = lambda: log.append({'call':'39388440 hydrate', 'bound':n.get(WORLD+0x10C)})
n.hooks[0x3938A770] = lambda: log.append({'call':'3938a770 replay', 'bound':n.get(WORLD+0x10C)})
n.hooks[0x39389360] = lambda: log.append({'call':'39389360 refresh', 'exclude':n.arg(0), 'bound':n.get(WORLD+0x10C)})
n.hooks[0x393FE030] = lambda: n.ret(pop=8)
def event():
    ui.append(n.arg(0)); n.ret()
n.hooks[0x39424220] = event

descs = {}
for i, (buff, has_skills) in enumerate([(5068,True),(8079,False),(6010,True)]):
    desc, vec, skill = 0x100D0000+i*0x1000, 0x100D0400+i*0x1000, 0x100D0500+i*0x1000
    n.put(desc+0x284,vec,vec+(4 if has_skills else 0))
    n.put(vec,skill); n.put(skill,20848+i)
    descs[buff]=desc
n.hooks[0x396F63D0] = lambda: n.ret(descs.get(n.arg(0),0))

# Callback presentation work is omitted, while its ORIGINAL mode-refresh gate
# 39389a2e..39389a62 executes in the live replay's stack/context.
def callback_gate():
    sp = n.u.reg_read(UC_X86_REG_ESP)
    provider, desc, row = n.u.reg_read(UC_X86_REG_ECX), n.arg(0), n.arg(1)
    gate_frames.append((n.u.reg_read(UC_X86_REG_EBP),n.u.reg_read(UC_X86_REG_EDI)))
    frame=sp-0x100
    n.put(frame-0x10,provider)
    n.u.reg_write(UC_X86_REG_EBP,frame); n.u.reg_write(UC_X86_REG_EDI,desc)
    log.append({'call':'39389440 callback presentation omitted; original tail gate',
                'buff':n.get(row+4),'instance':n.get(row),'bound':n.get(WORLD+0x10C),
                'owner':n.get(provider+0x20B8),'has_skills':n.get(desc+0x288)!=n.get(desc+0x284)})
    n.u.reg_write(UC_X86_REG_EIP,0x39389A2E)
def callback_return():
    ebp,edi=gate_frames.pop()
    n.u.reg_write(UC_X86_REG_EBP,ebp);n.u.reg_write(UC_X86_REG_EDI,edi)
    n.ret(pop=12)
n.hooks[0x39389440]=callback_gate
n.hooks[0x39389A62]=callback_return

n.put(SNAP,OBJ)
n.put(SNAP+0x438,0)
n.put(SNAP+0x440,CHAR)
n.u.mem_write(BUFFS,bytes(0x1600))
for count_offset, rows in [(0,[(1,5068),(2,8079)]),(0xA08,[(3,6010)]),(0x1190,[(4,999999)])]:
    n.u.mem_write(BUFFS+count_offset,bytes([len(rows)]))
    for i,(instance,buff) in enumerate(rows): n.put(BUFFS+count_offset+8+i*0x50,instance,buff)

def handler_body(end):
    # The pinned file has an unresolved protection detour at 391b90a3. Enter its
    # intact body with a fixture frame rather than editing or executing that stub.
    frame=0x201E0000
    n.put(frame+8,SNAP,BUFFS)
    n.u.reg_write(UC_X86_REG_EBP,frame)
    n.call(0x391B90BF,0,end=end)

results=[]
handler_body(0x391B93EC)
assert registry[0] and n.get(WORLD+0x10C)==actor[0]
assert ui==[539,95],ui
assert len([x for x in log if x['call']=='3938a770 replay'])==1
assert len([x for x in log if x['call'].startswith('39389440')])==3
assert len([x for x in log if x['call']=='39389360 refresh'])==1
results.append({'case':'initial_own_missing_object_model_descriptor_sink_returns_none','trace':list(log),'ui':list(ui)})

log.clear();ui.clear()
handler_body(0x391B950C)
assert len(log)==1 and log[0]['call']=='39360020 lookup' and not ui
results.append({'case':'same_own_SCUnitState_existing_object','trace':list(log),'ui':list(ui)})

log.clear();ui.clear()
n.call(0x39364DB0,actor[0])
assert ui==[539,95,539,95],ui
results.append({'case':'actual_OnModelChanged_after_binding','trace':list(log),'ui':list(ui)})

log.clear();ui.clear()
n.put(WORLD+0x10C,0)
n.call(0x39364DB0,actor[0])
assert not ui
results.append({'case':'actual_OnModelChanged_before_binding','trace':list(log),'ui':list(ui)})

log.clear();ui.clear();registry[0]=False
n.u.mem_write(BUFFS,bytes(0x1600))
handler_body(0x391B93EC)
assert ui==[539,95]
assert not [x for x in log if x['call'].startswith('39389440')]
results.append({'case':'initial_own_empty_lists_still_one_binding_publication','trace':list(log),'ui':list(ui)})

log.clear();ui.clear()
n.u.mem_write(BUFFS,b'\1');n.put(BUFFS+8,99,5068)
handler_body(0x391B950C)
assert len(log)==1 and not ui and n.get(actor[0]+0x3E80+0x20)==0
results.append({'case':'repeated_own_changed_payload_also_ignored_by_existing_object_branch','trace':list(log),'ui':list(ui)})

result={'game_sha256':SHA,'cases':results,
        'executed':['391b90bf handler body','3936b580 construction caller','3936b000 InitFromNetUnit',
                    '39388440 raw provider hydration','3938a770 full replay loop','39389a2e callback original eligibility tail',
                    '39355b60 local-owner identity','39187180 full binding body','39389360 full publication',
                    '392045f0 full modebar rebuild','39364db0 full OnModelChanged body'],
        'boundaries':['391b90a3 protected entry detour unresolved; intact handler body entered with fixture frame',
                      '39367d30 actor constructor substituted, no renderer/model/device created',
                      '398a1f90 non-buff unit-field hydration substituted',
                      '39389440 presentation callback body omitted; original refresh eligibility tail executed',
                      '39387e00 aggregate presentation recomputation and 39387c30 replay finalizer substituted',
                      '39695a70 returns no model descriptor: automatic model-callback scheduling/timing NOT proved',
                      'remaining unrelated manager/presentation/service endpoints substituted, addresses recorded',
                      'catalog fixture: 5068 and 6010 each have one skill; 8079 empty; 999999 missing'],
        'sink_addresses_seen':sorted(set(sink_calls)),
        'not_claimed':'No wire parser/protection stub execution, live client/server proof or automatic OnModelChanged scheduling proof.'}
(OUT/'snapshot-counts-cases.json').write_text(json.dumps(result,indent=2))
print(json.dumps({'passed':len(results),'cases':[{'name':x['case'],'ui':x['ui']} for x in results]}))
