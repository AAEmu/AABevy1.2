"""Isolated original-instruction follow-up. Read-only original DLL/SQLite inputs.
No live process/server/network. Boundary substitutions recorded with results.
"""
import hashlib
import json
from pathlib import Path
import sqlite3
import struct
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from user_music_lifecycle_oracle import Native,PLAYER,WORLD,RUNTIME_DATA,RUNTIME_SHA
from unicorn.x86_const import UC_X86_REG_EBP,UC_X86_REG_EDI,UC_X86_REG_EBX,UC_X86_REG_ESI
from capstone import Cs,CS_ARCH_X86,CS_MODE_32

OUT=Path(__file__).parent / "evidence" / "user_music_actions"
OUT.mkdir(parents=True, exist_ok=True)
PROVIDER=PLAYER+0x3e80
BAR,VECTOR=0x100b0000,0x100c0000
LISTS=[(0x20,0x28),(0xd28,0xd30),(0x16f0,0x16f8)]
RUNTIME=RUNTIME_DATA.read_bytes()
assert hashlib.sha256(RUNTIME).hexdigest()==RUNTIME_SHA

def constant(n,a):
    v=struct.unpack_from('<I',RUNTIME,a-0x39ac3000)[0];n.put(a,v);return v

def setup():
    n=Native()
    for a in [0x39d1fb9c,0x39ac62bc,0x3a1744d0,0x3a1744d4,0x3a1744e0,0x3a1744e8,0x39b02334,0x39d21164,0x39d2116c,0x39ac65bc,0x39ac65c0]:constant(n,a)
    n.put(0x39affa38,BAR);n.put(0x3a1744f0,VECTOR,VECTOR,VECTOR+0x1000)
    n.put(0x3a174880,1) # Already-created native profiling object; inactive ENV+97.
    n.hooks[0x39360020]=lambda:n.ret(0)
    n.hooks[0x39204500]=lambda:n.ret()
    n.put(PROVIDER,0x399d1990);n.put(PROVIDER+0x20b8,PLAYER)
    return n

def loader(db):
    n=Native();gdb=0x100b0000
    rows=db.execute('SELECT buff_id, skill_id FROM buff_skills').fetchall()
    buffs=sorted({r[0] for r in rows if db.execute('SELECT 1 FROM buffs WHERE id=?',(r[0],)).fetchone()})
    skills=sorted({r[1] for r in rows if db.execute('SELECT 1 FROM skills WHERE id=?',(r[1],)).fetchone()})
    descs={};skillptr={}
    for offset,ids in [(0x2878,buffs),(0x26d8,skills)]:
        base=0x100d0000 if offset==0x2878 else 0x100e0000
        n.put(gdb+offset,base,base+12*len(ids))
        for j,id in enumerate(ids):
            pointer=(0x10100000+j*0x300) if offset==0x2878 else (0x10200000+j*0x20)
            ptrcell=base+0x4000+4*j
            n.put(base+12*j,id,id,ptrcell);n.put(ptrcell,pointer);n.put(pointer,id)
            if offset==0x2878:
                descs[id]=pointer
                vector=0x100f0000+j*0x100
                n.put(pointer+0x284,vector,vector,vector+0x100)
            else:skillptr[id]=pointer
    position=[-1];query=[]
    def prepare():
        query.append(n.string(n.arg(1)).decode());n.put(n.arg(3),0x1234);n.ret(0)
    def step():
        position[0]+=1;n.ret(100 if position[0]<len(rows) else 101)
    n.hooks[0x3955b3d0]=prepare;n.hooks[0x3955fb10]=step
    n.hooks[0x3953a390]=lambda:n.ret(rows[position[0]][n.arg(1)])
    n.hooks[0x395519b0]=lambda:n.ret(0)
    assert n.call(0x395b16e0,0,777,gdb)&255==1
    assert query==['SELECT buff_id, skill_id FROM buff_skills']
    result={}
    for id,p in descs.items():
        a,z=n.get(p+0x284),n.get(p+0x288)
        actual=[n.get(n.get(at)) for at in range(a,z,4)]
        expected=[s for b,s in rows if b==id and s in skillptr]
        assert actual==expected,(id,actual,expected)
        result[id]=actual
    skipped=[dict(zip(['rowid','id','buff','skill','missing_buff','missing_skill'],r)) for r in db.execute('SELECT bs.rowid,bs.id,bs.buff_id,bs.skill_id,b.id IS NULL,s.id IS NULL FROM buff_skills bs LEFT JOIN buffs b ON b.id=bs.buff_id LEFT JOIN skills s ON s.id=bs.skill_id WHERE b.id IS NULL OR s.id IS NULL')]
    return {'native_sql':query[0],'input_rows':len(rows),'appended':n.get(gdb+0x10acc),'skipped_missing_catalog_rows':skipped,'vectors':{str(k):result[k] for k in [3530,4438,5068,6010]},'duplicate_preserved':result[4438].count(19167)==2,'boundaries':['actual SQLite rows supplied to native SQLite endpoints; controlled native buff/skill descriptor range tables and vector capacity; original lookup/append loop executes unchanged']}

def mode_refresh():
    n=setup();desc=0x100d0000;ptrs=desc+0x400
    n.put(desc+0x284,ptrs,ptrs+4);n.put(ptrs,ptrs+0x100);n.put(ptrs+0x100,20848)
    n.hooks[0x396f63d0]=lambda:n.ret(desc)
    calls=[]
    def ui():
        calls.append({'event':n.arg(0),'skills':[n.get(at) for at in range(n.get(0x3a1744f0),n.get(0x3a1744f4),4)],'mode_count':n.get(BAR+0x13b0)});n.ret()
    n.hooks[0x39424220]=ui
    n.hooks[0x393fe030]=lambda:(calls.append({'mode':n.arg(0),'enabled':n.arg(1)}),n.ret(pop=8))
    n.put(PROVIDER+0x20,1);n.put(PROVIDER+0x28,7,5068)
    results=[]
    for name,owner,count in [('own_new',PLAYER,1),('own_unchanged_again',PLAYER,1),('other_actor',PLAYER+0x500,1),('own_no_skill_rows',PLAYER,0)]:
        n.put(PROVIDER+0x20b8,owner);n.put(desc+0x288,ptrs+4*count)
        frame=0x201e0000;n.put(frame-0x10,PROVIDER)
        n.u.reg_write(UC_X86_REG_EBP,frame);n.u.reg_write(UC_X86_REG_EDI,desc)
        calls.clear();n.call(0x39389a2e,0,end=0x39389a62)
        if owner==PLAYER and count:
            assert [c.get('event') for c in calls if 'event'in c]==[539,95]
            assert calls[0]['skills']==[20848]
        else:assert not calls
        results.append({'case':name,'observations':list(calls)})
    # Exclusion applies to every equal instance across kind lists, before physical erase.
    n.put(desc+0x288,ptrs+4);n.put(PROVIDER+0x20b8,PLAYER)
    for count,start in LISTS:n.put(PROVIDER+count,1);n.put(PROVIDER+start,7,5068)
    first=n.call(0x39387440,PROVIDER,7)
    assert first==PROVIDER+0x28
    calls.clear();n.call(0x39389360,0,7)
    excluded_count=n.call(0x393871a0)
    assert excluded_count==0
    def memmove():
        dst,src,size=n.arg(0),n.arg(1),n.arg(2)
        if size:n.u.mem_write(dst,bytes(n.u.mem_read(src,size)))
        n.ret(dst)
    n.put(0x38ff6700,0x30000300);n.hooks[0x30000300]=memmove
    n.call(0x39387000,PROVIDER,first)
    remaining=[n.get(PROVIDER+c) for c,_ in LISTS]
    assert remaining==[0,1,1]
    calls.clear();n.call(0x39389360,0,0)
    assert n.call(0x393871a0)==2
    collision={'pre_erase_excluded_count':excluded_count,'actual_erase_counts':remaining,'later_refresh_no_exclusion_count':n.call(0x393871a0),'boundaries':['controlled raw provider rows; original traversal/find/erase executed; CRT memmove substituted']}
    # Cap is remaining space after earlier/base mode entries, not 15 buff actions.
    n.put(0x3a1744f0,VECTOR,VECTOR+32)
    for j in range(8):n.put(VECTOR+4*j,20848+j)
    cap=[]
    for base in [0,1,12,14,15]:
        n.put(BAR+0x13b0,base);n.call(0x392044a0,BAR)
        count=n.get(BAR+0x13b0);assert count==min(15,base+8)
        cap.append({'base_actions':base,'total':count,'appended':count-base})
    return {'receipt_gates':results,'collision':collision,'cap_after_base':cap}

def targets():
    n=setup();selected=[0];seen=[]
    def selected_provider():
        assert n.arg(2)==4
        n.put(n.arg(0),selected[0]);n.ret(n.arg(0))
    n.hooks[0x3a6779eb]=selected_provider
    def cast():
        ptr=n.arg(3)
        seen.append({'actor_pointer':n.arg(0),'skill':n.arg(1),'target_kind':n.get(ptr),'target_obj':n.get(ptr+8)})
        n.ret(1,pop=20)
    n.hooks[0x393989e0]=cast
    results=[]
    for name,selection,doodad,mask in [('selected_unit',99,0,0),('none',0,0,0),('fallback_doodad',0,777,0),('self_modifier',99,777,0x44)]:
        selected[0]=selection;n.put(PLAYER+0x5f90,doodad);seen.clear()
        n.call(0x39209d80,0,22213,0,0,47,0,0,0,0,0,mask,0,0)
        expected=(0,99) if selection and not mask else (4,777) if doodad and not mask else (0,47) if mask else (0,0)
        assert (seen[0]['target_kind'],seen[0]['target_obj'])==expected,(name,seen)
        results.append({'case':name,'native_cast_call':seen[0]})
    return {'cases':results,'boundaries':['current-selected-target primitive endpoint supplies controlled values; original target-choice branches and cast-call preparation execute; final393989e0 is an observing sink, not a full validation/network test']}

def slot246():
    n=setup();context=0x100d0000;calls=[]
    n.put(BAR,20848);n.put(BAR+0x14c,1)
    n.put(BAR+0x150,22213);n.put(BAR+0x150+0x14c,1)
    def skill():
        calls.append({'generic_skill':n.arg(0),'second_argument':n.arg(1)});n.ret(1,pop=8)
    n.hooks[0x392c5790]=skill;n.hooks[0x392c4dd0]=lambda:n.ret()
    result=[]
    for index in [1,2,15,16]:
        frame=0x201e0000;n.put(frame+12,index)
        n.u.reg_write(UC_X86_REG_EBP,frame);n.u.reg_write(UC_X86_REG_EDI,246);n.u.reg_write(UC_X86_REG_ESI,context)
        calls.clear();n.call(0x392c6c1d,0,end=0x392c6c80)
        expected=[{'generic_skill':20848 if index==1 else 22213,'second_argument':0}] if index<=2 else []
        assert calls==expected,(index,calls)
        result.append({'ui_index':index,'calls':list(calls)})
    return {'cases':result,'boundaries':['original246 switch/index conversion and kind1 mode-entry dispatch execute; final generic action setup392c5790 and tooltip refresh are sinks; target-choice core tested separately']}

def sources(n):
    blocks=[(0x395b16e0,0x252),(0x39389a2e,0x34),(0x39389c23,0x30),(0x39389360,0x65),
            (0x39355b60,0x18),(0x3938a770,0x161),(0x391d5e00,0x1ac),(0x39389fe1,0x1b),
            (0x393874d8,0x77),(0x39387000,0xd2),(0x392c6410,0x95),(0x392c6c1d,0x72),
            (0x392c5790,0x294),(0x3920a220,0xf3),(0x39209d80,0x176),
            (0x39214911,0xf),(0x39216315,0xf),(0x3920fa60,0x28a),(0x3920cc70,0x72),
            (0x391872bb,0x32),(0x3936b114,0x26),(0x3936b3fa,0xb),(0x39365139,0x14),
            (0x3938a512,0x38),(0x393886c0,0x1b5)]
    cs=Cs(CS_ARCH_X86,CS_MODE_32);ss=['dll_sha256='+n.images[0][1]]
    for addr,size in blocks:
        raw=bytes(n.u.mem_read(addr,size));ss.append(f'\nSOURCE {addr:08x} length={size} sha256={hashlib.sha256(raw).hexdigest()}')
        ss.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}'.rstrip() for i in cs.disasm(raw,addr))
    (OUT/'followup-native-source.txt').write_text('\n'.join(ss)+'\n')

def main():
    db=sqlite3.connect('file:D:/aaemu-live/rs-game/Data/compact.sqlite3?mode=ro',uri=True);db.execute('PRAGMA query_only=ON')
    result={'loader':loader(db),'refresh':mode_refresh(),'targets':targets(),'slot246':slot246(),
            'event_names':{'539':{'name':'BUFF_SKILL_CHANGED','name_initializer':'39216315','name_table_slot':'39d23554'},'95':{'name':'MODE_ACTIONS_UPDATE','name_initializer':'39214911','name_table_slot':'39d22e64'}},
            'event_name_table_base':'39d22ce8; original3920faf8 reads base+4*event_id',
            'limitations':'Isolated native instruction execution and read-only sources only; no live original client/server/network validation.'}
    sources(Native());(OUT/'followup-cases.json').write_text(json.dumps(result,indent=2)+'\n')
    print('Passed native loader567 rows, refresh gates, duplicate-ID exclusion/erase, cap-after-base, target-choice cases')

if __name__=='__main__':main()
