"""Bounded native 1.2 forced-motion / seating instruction oracle.

Only Unicorn executes code; no DLL loading, game process, network, device or build.
All writes are fixture memory or this ignored subdirectory. Endpoint substitutions
are listed in report.txt. Parent seat evidence is read and hash-checked only.
Run from repo root: python -B data/extract/aa-client/seat-presentation/forced-movement/oracle.py
"""
from pathlib import Path
import hashlib
import json
import struct
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *

OUT = Path(__file__).resolve().parent
GAME = Path('<client12-unpacked>/x2game_unpacked.dll')
ENTITY = Path('<client>/bin32/cryentitysystem.dll')
RUNTIME = Path('<re-notes>/housing-audit/native12-data-readonly.bin')
SHAS = {
    str(GAME): '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e',
    str(ENTITY): 'd1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e',
    str(RUNTIME): '7082573df48d7d5229e1f909845733f0cf9ded6ea1a4e3c41a5e276fccde8c9e',
}
U,A,AV,E,EV,ATT,PARENT,C,CV,W,ENV,TIMER,TV,I,DESC,P,POS,Q,PHYS,PV,DEST,STATE = [0x10010000 + i*0x10000 for i in range(22)]
STOP = 0x3000fff0
ACTOR, OWNER = 47, 123
CASES = []


class VM:
    def __init__(self):
        self.u = Uc(UC_ARCH_X86, UC_MODE_32)
        for path in (GAME, ENTITY):
            data = path.read_bytes()
            assert hashlib.sha256(data).hexdigest() == SHAS[str(path)]
            pe = pefile.PE(data=data)
            image = pe.get_memory_mapped_image()
            self.u.mem_map(pe.OPTIONAL_HEADER.ImageBase, (len(image)+4095)&~4095)
            self.u.mem_write(pe.OPTIONAL_HEADER.ImageBase, image)
        for a,size in [(0,0x1000),(0x10000000,0x400000),(0x20000000,0x200000),(0x30000000,0x10000)]:
            self.u.mem_map(a,size)
        runtime = RUNTIME.read_bytes()
        assert hashlib.sha256(runtime).hexdigest() == SHAS[str(RUNTIME)]
        self.constants = {}
        for a in [0x39ac304c,0x39ac3050,0x39ac3054,0x39b022a0,0x3a1699f4,
                  0x3a169968,0x39ae6d14,0x39ae6d24,0x39ae6d30,0x39ae6d34,
                  0x39ae6d38,0x39ac38c8,0x39ac7464,0x39ac7468,0x39ac746c,0x39ac7470,
                  0x3a177710,0x3a17772c,0x39ae6c20,0x39ae6c24,0x39ae6c28,0x39ae6c2c,0x39ae4f40]:
            self.put(a,struct.unpack_from('<I',runtime,a-0x39ac3000)[0])
            self.constants[f'{a:08x}'] = self.get(a)
        self.hooks = {}
        self.events = []
        self.trace = []
        self.heap = 0x10300000
        self.now = 100
        self.entity_present = self.target_present = self.physical_present = True
        self.actor_class = 1
        self.put(U+8,ACTOR)
        self.put(U+0x3be8,A)
        self.put(A,AV)
        self.put(A+0xc,1)
        self.put(A+0x1f0,C)
        self.put(A+0x164,0x10200000)
        self.put(C,CV)
        self.put(C+0xc,E)
        self.put(E,EV)
        self.put(E+0xa0,ATT)
        self.put(ATT+0x10,PARENT)
        self.floats(E+0x28,1,2,3)
        self.floats(E+0x34,0,0,0,1)
        self.floats(E+0x50,1,0,0,1,0,1,0,2,0,0,1,3)
        self.floats(E+0x80,0,0,0,1,1,2,3,1)
        self.put(PHYS,PV)
        self.put(W+0x10c,U)
        self.put(0x39b02294,W)
        self.put(0x39aff9d0,I)
        self.put(0x3a5bb1d8,ENV)
        self.put(ENV+0x14,TIMER)
        self.put(TIMER,TV)
        self.put(I,ACTOR)
        self.put(I+0xc4,STATE)
        self.put(STATE,self.get(0x3a59f858))
        # These vtable entries execute the original getter/setter instructions.
        for off,target in [(0x20,0x390e2d80),(0x24,0x390bf990),(0x28,0x390bf9d0),(0x34,0x390e2fc0),
                           (0x58,0x390c0a00),(0x8c,0x390e2f90),(0x98,0x390ca040),
                           (0x200,0x390c0190),(0x23c,0x390c1420),(0x248,0x390c4070),
                           (0x274,0x390c15c0)]:
            self.put(AV+off,target)
        for off,target in [(0x50,0x32016b20),(0x60,0x3200e160),(0x64,0x3200e170),
                           (0x78,0x3200f740),(0x80,0x32010d70)]:
            self.put(EV+off,target)
        self.endpoint(AV+4,lambda:self.ret(self.actor_class))
        self.endpoint(AV+0x1b8,lambda:self.ret(E if self.entity_present else 0))
        self.endpoint(AV+0x208,lambda:self.ret(C))
        self.endpoint(AV+0x228,lambda:self.ret(Q))
        self.endpoint(AV+0x1a0,lambda:self.ret(PHYS if self.physical_present else 0))
        self.endpoint(AV+0x44,lambda:self.event_ret('actor_refresh'))
        self.endpoint(EV+0x1a4,lambda:self.event_ret('entity_invalidate',pop=4,flags=self.arg(0)))
        self.endpoint(EV+0x44,self.detach)
        self.endpoint(TV+0x10,lambda:self.ret(self.now,pop=4))
        self.endpoint(PV+0x28,self.physics)
        self.endpoint(AV+0x120,self.animation)
        self.endpoint(AV+0x124,lambda:self.event_ret('stop_animation',pop=4,handle=self.arg(0)))
        self.hooks.update({
            0x39360020:lambda:self.ret(U if self.target_present and self.arg(0)==ACTOR else 0),
            0x39035110:lambda:self.ret(0,pop=4), # owner lookup absent: omit optional boat packet
            0x39186810:self.packet,
            0x3900b590:lambda:self.event_ret('cancel_bond_callback',pop=8,bond=self.bond()),
            0x39155ed0:lambda:self.event_ret('graph_action',pop=12,action=self.arg(0)),
            0x390115a0:lambda:self.ret(pop=16), # profiler begin
            0x390116d0:lambda:self.ret(), # profiler end
            0x3950a1b0:self.allocate,
            0x390bdb30:lambda:self.event_ret('input_lock_notify'),
            0x390ca140:lambda:self.event_ret('controller_notify'),
            0x393966d0:lambda:self.event_ret('activity_side_effect_393966d0'),
            0x393967c0:lambda:self.event_ret('activity_side_effect_393967c0'),
            0x39016c40:lambda:self.event_ret('activity_side_effect_39016c40'),
            0x394fab30:lambda:self.event_ret('activity_side_effect_394fab30'),
        })
        self.u.hook_add(UC_HOOK_CODE,self.hook)
        self.active()
        self.floats(POS,11,12,13)
        self.floats(Q,0,0,0,1,1,2,3,1)

    def put(self,a,*values): self.u.mem_write(a,struct.pack('<'+'I'*len(values),*(v&0xffffffff for v in values)))
    def get(self,a): return struct.unpack('<I',self.u.mem_read(a,4))[0]
    def floats(self,a,*values): self.u.mem_write(a,struct.pack('<'+'f'*len(values),*values))
    def fs(self,a,n): return list(struct.unpack('<'+'f'*n,self.u.mem_read(a,4*n)))
    def arg(self,i): return self.get(self.u.reg_read(UC_X86_REG_ESP)+4+4*i)
    def ret(self,value=0,pop=0):
        sp=self.u.reg_read(UC_X86_REG_ESP)
        self.u.reg_write(UC_X86_REG_EAX,value&0xffffffff)
        self.u.reg_write(UC_X86_REG_EIP,self.get(sp))
        self.u.reg_write(UC_X86_REG_ESP,sp+4+pop)
    def event_ret(self,event,pop=0,**data):
        self.events.append({'endpoint':event,**data})
        self.ret(pop=pop)
    def endpoint(self,slot,hook):
        addr=0x30001000+len(self.hooks)*0x10
        self.hooks[addr]=hook
        self.put(slot,addr)
    def hook(self,u,a,size,data):
        if a in [0x3935a520,0x393566f0,0x390b3930,0x390c4070,0x390e2f90,0x39405dfa,0x39405f93]:
            self.trace.append(f'{a:08x}')
        if a in self.hooks: self.hooks[a]()
    def call(self,a,this=0,*args,end=STOP):
        self.put(0x201ff000,STOP,*args)
        self.u.reg_write(UC_X86_REG_ESP,0x201ff000)
        self.u.reg_write(UC_X86_REG_ECX,this)
        self.u.emu_start(a,end,count=200000)
        assert self.u.reg_read(UC_X86_REG_EIP)==end,hex(self.u.reg_read(UC_X86_REG_EIP))
    def slice(self,a,b,**regs):
        for k,v in regs.items(): self.u.reg_write(globals()['UC_X86_REG_'+k],v)
        self.u.emu_start(a,b,count=100000)
        assert self.u.reg_read(UC_X86_REG_EIP)==b,hex(self.u.reg_read(UC_X86_REG_EIP))
    def active(self):
        self.put(U+0x3b94,OWNER,0,1,2,2)
        self.put(A+0x160,OWNER)
    def bond(self): return [self.get(U+0x3b94+i*4) for i in range(5)]
    def detach(self):
        self.events.append({'endpoint':'entity_detach','args':[self.arg(0),self.arg(1)],'bond':self.bond()})
        self.put(ATT+0x10,0) # hierarchy endpoint model only, no physics or world-matrix computation
        self.ret(pop=8)
    def packet(self):
        p=self.arg(0)
        op=self.get(p+4)
        self.events.append({'endpoint':'packet_sink','opcode':op,'body_fixture_dwords':[self.get(p+o) for o in (12,16,20)],'bond':self.bond()})
        self.ret(pop=4)
    def allocate(self):
        size=self.arg(0); at=self.heap; self.heap+=(size+15)&~15
        self.u.mem_write(at,bytes(size)); self.ret(at)
    def physics(self):
        p=self.arg(0)
        self.events.append({'endpoint':'physical_action','type':self.get(p),'arg1':self.arg(1),
                            'raw':bytes(self.u.mem_read(p,0x50)).hex(),'parent':self.get(ATT+0x10),'bond':self.bond()})
        self.ret(1,pop=8)
    def animation(self):
        p=self.arg(0)
        self.events.append({'endpoint':'play_animation','id':self.arg(1),'flags':self.arg(2)})
        self.put(p,501); self.ret(p,pop=28)
    def save(self,name,**extra):
        row={'case':name,'bond':self.bond(),'parent':self.get(ATT+0x10),'actor_attachment':self.get(A+0x11c),
             'actor_owner':self.get(A+0x160),'local_pos':self.fs(E+0x28,3),'local_quat':self.fs(E+0x34,4),
             'activity':list(self.u.mem_read(I+0xb8,2)),'controller':self.get(A+0x58),
             'trace':self.trace.copy(),'events':self.events.copy(),**extra}
        CASES.append(row); self.trace.clear(); self.events.clear()
    def input_producer_and_exit(self):
        # Isolated input-core tail; middle input sampling/state selection is NOT emulated.
        bp=0x201fe000
        self.u.mem_write(bp-0x300,bytes(0x300))
        self.put(bp-0x5c,A); self.put(bp-0x34,U)
        self.u.mem_write(I+0xb8,b'\0\0')
        self.u.reg_write(UC_X86_REG_XMM5,0)
        self.slice(0x394058ba,0x39405f98,EBP=bp,ESP=bp-0x400,EDI=I)


def position_cases():
    n=VM()
    for label,ap,ep in [('entity_parent_blocks',0,PARENT),('actor_attachment_blocks',PARENT,0),('both_block',PARENT,PARENT),('logical_bond_only_accepts',0,0)]:
        n.active();n.put(A+0x11c,ap);n.put(ATT+0x10,ep);n.floats(E+0x28,1,2,3)
        n.call(0x390b3930,U,POS)
        assert n.fs(E+0x28,3)==([1,2,3] if ap or ep else [11,12,13])
        assert n.bond()[0]==OWNER and not any('3935' in t for t in n.trace)
        n.save('position_setter_'+label)
    n.active();n.put(ATT+0x10,PARENT);n.floats(POS,0.70710677,0,0,0.70710677)
    n.call(0x32010d70,E,POS,0)
    assert n.fs(E+0x34,4)==n.fs(POS,4) and n.get(ATT+0x10)==PARENT
    n.save('entity_rotation_changes_local_quat_with_parent_retained')


def controller_cases():
    n=VM()
    n.put(DESC+0x10,4);n.put(DESC+0x14,n.get(0x39ae6d30));n.put(DESC+0x24,1000)
    for label,cls,flags in [('remote_class_rejects',2,1),('mask1_rejects',1,0),('local_dash_admitted',1,1)]:
        n.actor_class=cls;n.put(A+0xc,flags)
        n.call(0x390ca700,A,DESC,0,7)
        assert bool(n.get(A+0x58))==(label=='local_dash_admitted')
        assert n.bond()[0]==OWNER and not any(t in ['3935a520','393566f0'] for t in n.trace)
        n.save('controller_start_'+label,lock0=n.get(A+0x70),early_return_flag=n.get(C+0x1d0)&255)
    ctl=n.get(A+0x58)
    assert n.get(A+0x70)==1
    # Original input entry executes the early-return guard, before any middle input sampling.
    n.u.mem_write(C+0x1d0,b'\1')
    n.call(0x39404d50,I)
    assert n.bond()[0]==OWNER and list(n.u.mem_read(I+0xb8,2))==[1,0]
    n.save('controller_input_1d0_early_return_sets_B8_without_exit')
    n.u.mem_write(C+0x1d0,b'\0')
    n.input_producer_and_exit()
    assert n.bond()==[0,0xffffffff,0,0,1] and n.get(A+0x58)==ctl
    assert [e['endpoint'] for e in n.events if e['endpoint'] in ['packet_sink','entity_detach']]==['packet_sink','entity_detach']
    n.save('controller_input_tail_active_controller_exits_with_zero_user_input',lock0=n.get(A+0x70),slice_bridge='394058ba: zero sampled deltas/yaw, no state override')
    n.input_producer_and_exit()
    assert not any(e['endpoint']=='packet_sink' for e in n.events)
    n.save('controller_next_input_no_duplicate_CD_after_clear')
    n.active();n.put(ATT+0x10,PARENT);n.put(A+0x58,0)
    n.input_producer_and_exit()
    assert n.bond()[0]==OWNER
    n.save('input_tail_without_controller_or_user_input_keeps_bond')
    # Logical empty/rejected controller result is the actual activity producer's eligibility boundary.
    n.put(A+0x58,0x10310000)
    n.input_producer_and_exit()
    assert n.bond()[0]==0
    n.save('input_tail_any_nonnull_controller_pointer_triggers_B8')


def impulse_cases():
    n=VM()
    n.put(P+0x10,ACTOR)
    # SCUnitImpulse union: kind0 / same owner selects own actor quaternion; vectors kept distinct.
    n.put(P+0x18,0);n.put(P+0x20,ACTOR)
    for off,vec in [(0x28,(1,2,3)),(0x34,(4,5,6)),(0x40,(7,8,9)),(0x4c,(10,11,12))]:
        n.floats(P+0x10+off,*vec)
    for label,target,flags,physical in [('missing_target',False,1,True),('mask1_rejects',True,0,True),('no_physical_entity',True,1,False),('parented_physical_dispatch',True,1,True)]:
        n.target_present=target;n.physical_present=physical;n.put(A+0xc,flags)
        n.call(0x391d7750,0,W,P)
        actions=[e for e in n.events if e['endpoint']=='physical_action']
        assert bool(actions)==(target and flags==1 and physical)
        if actions:
            assert actions[0]['type']==2 and actions[0]['parent']==PARENT
            raw=bytes.fromhex(actions[0]['raw'])
            assert list(struct.unpack_from('<3f',raw,4))==[4,5,6]
            assert list(struct.unpack_from('<3f',raw,16))==[10,11,12]
            assert list(struct.unpack_from('<3f',raw,28))==[1,2,3]
            assert list(struct.unpack_from('<3f',raw,40))==[7,8,9]
        assert n.bond()[0]==OWNER and n.get(ATT+0x10)==PARENT and n.get(A+0x58)==0
        n.save('SCUnitImpulse_'+label,physical_effect='not emulated')
    # Controller-free input tail after impulse still has zero activity and no exit.
    n.input_producer_and_exit()
    assert n.bond()[0]==OWNER
    n.save('input_tail_after_impulse_does_not_infer_activity_from_displacement')


def controller_update_cases():
    n=VM()
    n.put(DESC+0x10,4);n.put(DESC+0x14,n.get(0x39ae6d30));n.put(DESC+0x24,1000)
    n.call(0x390ca700,A,DESC,0,7)
    n.events.clear();ctl=n.get(A+0x58)
    mc,mv=0x10250000,0x10251000
    n.put(CV+0xb0,0x39122710);n.put(C+0x2c,mc);n.put(mc,mv)
    n.endpoint(CV+0x80,lambda:n.event_ret('movement_component_v80',pop=4,arg=n.arg(0)))
    def movement():
        p=n.arg(0)
        n.events.append({'endpoint':'movement_controller_request','flags':n.get(p),'parent':n.get(ATT+0x10),'bond':n.bond(),'delta':n.fs(p+0x30,3)})
        n.ret(1,pop=4)
    n.endpoint(mv,movement)
    n.hooks[0x390a5d00]=lambda:n.ret() # request-owned string destruction only
    n.call(0x390a8ee0,ctl)
    assert n.bond()[0]==OWNER and any(e['endpoint']=='movement_controller_request' for e in n.events)
    request=next(e for e in n.events if e['endpoint']=='movement_controller_request')
    assert request['flags']==0x820 and request['delta']==[0,1,0]
    assert n.get(C+0x40)==0x800
    n.save('dash_update_requests_motion_before_separate_input_pass_without_detach',physics='not emulated')
    n.now=1100
    n.call(0x390a8ee0,ctl)
    assert n.u.mem_read(ctl+0x1c,1)==b'\1' and not n.events
    n.save('dash_update_expiry_marks_finished_but_pointer_still_present')
    n.input_producer_and_exit()
    assert n.bond()[0]==0
    n.save('finished_dash_pointer_still_causes_activity_until_controller_removed')

    n=VM()
    n.put(DESC+0x10,1);n.put(DESC+0x14,n.get(0x39ae6d30));n.put(DESC+0x24,6000);n.put(DESC+0x40,18000)
    n.call(0x390ca700,A,DESC,0,8)
    ctl=n.get(A+0x58)
    assert ctl and n.get(A+0x70)==0 and n.get(A+0x7c)==1 and n.get(A+0x80)==1
    n.save('float_controller_admitted_locks_3_and_4_without_immediate_exit')
    n.put(CV+0xb0,0x39122710) # null movement-controller branch after physical request
    n.call(0x390a9680,ctl)
    actions=[e for e in n.events if e['endpoint']=='physical_action']
    assert len(actions)==1 and actions[0]['type']==10 and n.bond()[0]==OWNER
    assert list(struct.unpack_from('<3f',bytes.fromhex(actions[0]['raw']),12))==[0,0,18]
    n.save('float_update_dispatches_physical_velocity_without_position_setter',physics='not emulated')
    n.input_producer_and_exit()
    assert n.bond()[0]==0
    n.save('float_controller_pointer_causes_B8_exit_without_user_input')
    n.active();n.put(ATT+0x10,PARENT)
    n.put(A+0x64,0x10210000);n.put(0x10210000,0x10210000) # empty observer tree
    n.put(DESC+8,n.get(0x39ae6d30));n.put(ctl+0xc,n.get(0x39ae4f40)) # no requested start/end animation
    n.hooks[0x3950a1f0]=lambda:n.event_ret('free_controller_fixture')
    n.call(0x390ca040,A,9)
    assert n.get(A+0x58)==ctl and not n.events
    n.save('float_end_mismatched_tag_keeps_current_controller')
    n.call(0x390ca040,A,8)
    assert n.get(A+0x58)==0 and n.get(A+0x7c)==0 and n.get(A+0x80)==0
    assert n.bond()[0]==OWNER and n.get(ATT+0x10)==PARENT
    n.save('float_end_matched_tag_removes_controller_without_detaching_bond')
    n.input_producer_and_exit()
    assert n.bond()[0]==OWNER and list(n.u.mem_read(I+0xb8,2))==[0,0]
    n.save('input_tail_after_float_controller_removed_has_no_controller_exit')


def cancellation_order_cases():
    n=VM()
    for a in (0x393966d0,0x393967c0,0x39016c40,0x394fab30): del n.hooks[a]
    n.put(0x39aff9ac,0x10220000);n.put(0x39aff9a8,0x10230000);n.put(0x39aff9b8,0x10240000)
    n.put(U+0x3ac4,11);n.put(U+0x3acc,12)
    n.put(0x10240000,0x00010000,888) # admitted nonempty loot owner; empty contents vector
    n.put(A+0x58,0x10310000)
    def stop_cast():
        tl=n.arg(0)
        n.events.append({'endpoint':'cast_stop_local','tl':tl,'bond':n.bond()})
        for off in (0x3ac4,0x3acc):
            if n.get(U+off)==tl: n.put(U+off,n.get(0x3a177710))
        n.ret(pop=8)
    n.hooks.update({0x39396000:stop_cast,
        0x3938ade0:lambda:n.event_ret('buff_move_filter'),
        0x3938afc0:lambda:n.event_ret('buff_move_filter_other'),
        0x393546f0:lambda:n.event_ret('clear_looting_action',pop=4),
        0x394f9ec0:lambda:n.ret(1,pop=4),
        0x39424220:lambda:n.event_ret('lua_event',event_id=n.arg(0)),
    })
    n.input_producer_and_exit()
    ops=[e['opcode'] for e in n.events if e['endpoint']=='packet_sink']
    assert ops==[0x54,0x54,0x90,0xcd],ops
    assert n.bond()[0]==0
    n.save('controller_B8_orders_054_cast_stops_then_090_loot_close_then_0CD_then_detach',
           boundaries='cast-stop local fold, buff move filters, Lua and loot action are endpoints; original packet constructors and send ordering execute')
    n.input_producer_and_exit()
    assert not any(e['endpoint']=='packet_sink' for e in n.events)
    n.save('already_cleared_cast_loot_bond_do_not_resend_054_090_CD')


def upstream_gate_cases():
    n=VM()
    skill,manager=0x10260000,0x10270000
    n.endpoint(AV+0x90,lambda:n.event_ret('start_controller_admission_boundary',pop=12,desc=n.arg(0),target=n.arg(1),tag=n.arg(2)))
    bp=0x201fe000
    for label,present,at_end,end_flag,root,kind,expected in [
        ('missing_descriptor',False,0,0,0,4,False),
        ('at_end_not_fire',True,1,0,0,4,False),
        ('end_controller_flag_not_start',True,0,1,0,4,False),
        ('root_rejects_dash',True,0,0,1,4,False),
        ('root_allows_leap',True,0,0,1,2,True),
        ('ordinary_dash_enters_admission',True,0,0,0,4,True),
    ]:
        n.put(skill+0x13c,DESC if present else 0);n.put(DESC+0x10,kind)
        n.u.mem_write(skill+0x140,bytes([at_end]));n.u.mem_write(skill+0xc0,bytes([end_flag]))
        n.u.mem_write(U+0x3a72,bytes([root]));n.put(bp+0x20,P)
        n.slice(0x3939b4ca,0x3939b574,EBP=bp,ESP=bp-0x400,EDI=skill,ESI=U)
        assert bool(n.events)==expected
        n.save('skill_fire_gate_'+label,boundary='v90 endpoint; geometry/controller construction tested separately')
    n.put(manager+0x20b8,U);n.put(bp-0x10,manager);n.put(skill+0x18c,DESC);n.put(DESC+0x10,2)
    for root in (1,0):
        n.u.mem_write(U+0x3a72,bytes([root]))
        n.slice(0x393898e4,0x39389956,EBP=bp,ESP=bp-0x400,EDI=skill,ESI=A,EBX=P)
        assert bool(n.events)==(root==0)
        n.save('buff_controller_gate_root_'+str(root),boundary='v90 endpoint; unlike skill fire, rooted buff Leap is not admitted')


def knockdown_cases():
    n=VM()
    for label,vec,anim in [('zero_direction',(0,0,0),269),('forward',(0,1,0),276),('back',(0,-1,0),269)]:
        n.floats(POS,*vec)
        n.call(0x390c0b80,A,0x3f800000,POS,3)
        assert [e['id'] for e in n.events if e['endpoint']=='play_animation']==[anim]
        assert n.bond()[0]==OWNER and n.get(ATT+0x10)==PARENT and n.get(A+0x58)==0
        n.save('knockdown_pose_'+label,root_motion='not emulated')


def blink_cases():
    n=VM()
    n.put(P+0xc,ACTOR)
    n.floats(P+0x10,0,0)
    n.floats(DEST,11,12,13)
    def decode():
        out=n.arg(0);n.floats(out,11,12,13);n.ret(out,pop=8)
    def collision():
        out=n.arg(0);n.floats(out,11,12,13)
        n.events.append({'endpoint':'blink_collision_destination_fixture','destination':n.fs(out,3),'flags':n.arg(5)})
        n.ret(out)
    def ownid():
        out=n.arg(0);n.put(out,ACTOR);n.ret(out,pop=4)
    n.hooks.update({0x390b3850:lambda:n.ret(E+0x28),0x3961de30:decode,0x390b8060:collision,
                    0x3915e930:lambda:n.event_ret('blink_view_endpoint'),0x39079600:ownid})
    for label,own,gate,target,parent,ap in [
        ('no_own_unit',False,False,True,PARENT,0),
        ('own_3af0_gate_rejects',True,True,True,PARENT,0),
        ('missing_target',True,False,False,PARENT,0),
        ('entity_parent_translation_rejected',True,False,True,PARENT,0),
        ('actor_attachment_translation_rejected',True,False,True,0,PARENT),
        ('logical_bond_only_translation_accepted',True,False,True,0,0),
    ]:
        n.put(W+0x10c,U if own else 0)
        n.put(U+0x3af0,n.get(0x39b022a0)+(1 if gate else 0))
        n.target_present=target;n.put(ATT+0x10,parent);n.put(A+0x11c,ap)
        n.floats(E+0x28,1,2,3);n.floats(E+0x34,0,0,0,1)
        n.floats(Q,0,0,0.70710677,0.70710677,1,2,3,1)
        n.call(0x391d7050,0,W,P)
        eligible=own and not gate and target
        assert n.fs(E+0x28,3)==([11,12,13] if eligible and not parent and not ap else [1,2,3])
        assert n.fs(E+0x34,4)==(n.fs(Q,4) if eligible else [0,0,0,1])
        assert n.bond()[0]==OWNER and not any(t in ['3935a520','393566f0'] for t in n.trace)
        n.save('SCBlinkUnit_'+label,geometry='decode/collision fixtures; same-id branch',view_effect='endpoint only')


def verify_prior():
    baseline=json.loads((OUT/'baseline.json').read_text())
    for row in baseline['prior_files']:
        data=(OUT.parent/row['name']).read_bytes()
        assert len(data)==row['bytes'] and hashlib.sha256(data).hexdigest()==row['sha256'],row['name']
    return len(baseline['prior_files'])


def main():
    count=verify_prior()
    position_cases();controller_cases();controller_update_cases();cancellation_order_cases();upstream_gate_cases();impulse_cases();knockdown_cases();blink_cases()
    result={'modules':SHAS,'case_count':len(CASES),'prior_files_unchanged':verify_prior(),
            'prior_case_counts':[13,50],'cases':CASES}
    (OUT/'cases.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(f'{len(CASES)} isolated native cases passed; {count} prior files unchanged')


if __name__=='__main__': main()
