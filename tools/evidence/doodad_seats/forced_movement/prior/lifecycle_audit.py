"""Isolated original-instruction seat lifetime probes; no game/process/socket/device.

Run: python -B data/extract/aa-client/seat-presentation/lifecycle_audit.py
The inherited Native class is used only for PE mapping and x86 ABI utilities;
install() is overridden, so none of its unrelated endpoint fixtures run.
All writes are Unicorn fixture memory or this ignored directory.
"""
from pathlib import Path
import hashlib
import json
import struct
import sys
import pefile
from unicorn.x86_const import *

OUT = Path(__file__).resolve().parent
sys.path.insert(0, str(OUT.parents[3] / 'tools'))
from user_music_lifecycle_oracle import Native, SHA, RUNTIME_DATA, RUNTIME_SHA

ENTITY = Path('D:/AA1.2/bin32/cryentitysystem.dll')
ENTITY_SHA = 'd1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e'
U, A, AV, D, DV, R, RV, E, EV, C, CV, B, OCC = [0x10070000+i*0x10000 for i in range(13)]
W, ENV, NODE = 0x10040000, 0x10041000, 0x10042000
ACTOR, OWNER = 47, 123


class Probe(Native):
    def install(self):
        pass

    def __init__(self):
        super().__init__()
        runtime = RUNTIME_DATA.read_bytes()
        assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
        for a in [0x39ac38c8,0x39ae6d14,0x39ae6d38,0x3a169968,0x3a1699f4,
                  0x3a5e128c,0x39ac7464,0x39ac7468,0x39ac746c,0x39ac7470,
                  0x39afd36c,0x39ac3f20,0x39ac3f24]:
            self.put(a, struct.unpack_from('<I',runtime,a-0x39ac3000)[0])
        self.events = []
        self.owner_present = self.render_present = self.entity_present = True
        self.actor_present = True
        self.callback_actor_present = True
        self.put(0x39b02294, W)
        self.put(W+0x10c,U)
        self.put(0x3a5bb1d8, ENV)
        self.put(U+8,ACTOR)
        self.put(U+0x3be8,A)
        for o,v in [(A,AV),(D,DV),(R,RV),(E,EV),(C,CV)]: self.put(o,v)
        self.put(A+0x164,0x10063000)
        self.put(D+0x80,1236)
        self.put(D+0x364,OCC)
        self.put(OCC,2,2,1,0,0,0,0,0,1236)
        self.hooks.update({
            0x39360020:lambda:self.ret(U if self.actor_present and self.arg(0)==ACTOR else 0),
            0x39035110:lambda:self.ret(D if self.owner_present and self.arg(0)==OWNER else 0,pop=4),
            0x390b4990:self.character_id,
            0x39360920:lambda:self.ret(),
            0x390b3750:lambda:self.ret(A),
            0x3900c690:lambda:None if self.arg(1)==0 else self.queue(),
            0x3900b590:self.cancel,
            0x39186810:self.packet,
            0x3935f560:self.resolve_callback_actor,
            0x39155ed0:self.action,
        })
        self.endpoint(DV+0xc,lambda:self.ret(R if self.render_present else 0))
        self.endpoint(RV+0x68,lambda:self.ret(C if self.entity_present else 0))
        self.endpoint(AV+4,lambda:self.ret(1))
        self.endpoint(AV+0x1b8,lambda:self.ret(E))
        self.endpoint(AV+0x238,self.attach,pop_unused=True)
        self.put(AV+0x23c,0x390c1420)
        self.endpoint(AV+0x44,lambda:(self.events.append({'refresh_actor':True}),self.ret()))
        self.endpoint(EV+0x44,self.detach)
        self.put(B,OWNER,0,1,2,2)
        self.put(0x38ff6f34,self.fn(lambda:self.ret()))

    def fn(self,hook):
        address=0x30004000+len(self.hooks)*0x10
        self.hooks[address]=hook
        return address

    def endpoint(self,slot,hook,**_): self.put(slot,self.fn(hook))

    def character_id(self):
        self.put(self.arg(0),42)
        self.ret(self.arg(0),pop=4)

    def attach(self):
        self.events.append({'apply_actor_bond_endpoint':list(struct.unpack('<5I',self.u.mem_read(self.arg(0),20))),
                            'doodad':self.arg(1)})
        self.put(A+0x160, self.get(self.arg(0)))
        self.ret(pop=8)

    def detach(self):
        self.events.append({'entity_detach_endpoint':[self.arg(0),self.arg(1)],'bond_owner_at_detach':self.get(U+0x3b94)})
        self.ret(pop=8)

    def queue(self):
        self.events.append({'queue_endpoint':[self.arg(i) for i in range(5)]})
        self.ret(pop=20)

    def cancel(self):
        self.events.append({'cancel_callback_endpoint':[self.arg(0),self.arg(1)],'bond_owner_at_cancel':self.get(U+0x3b94)})
        self.ret(pop=8)

    def resolve_callback_actor(self):
        self.put(self.arg(0), NODE if self.callback_actor_present else 0)
        self.put(NODE+0x10,U)
        self.ret(self.arg(0),pop=8)

    def packet(self):
        p=self.arg(0)
        self.events.append({'packet_sink':hex(self.get(p+4)),'actor':self.get(p+12),'owner':self.get(p+16)})
        self.ret(pop=4)

    def action(self):
        self.events.append({'action_endpoint':[self.arg(i) for i in range(3)]})
        self.ret(pop=12)

    def bond(self): return list(struct.unpack('<5I',self.u.mem_read(U+0x3b94,20)))

    def received(self): self.call(0x393b2b90,0,ACTOR,B)

    def active(self):
        self.put(U+0x3b94,OWNER,0,1,2,2)
        self.put(A+0x160,OWNER)

    def slice(self,a,b,**regs):
        for key,value in regs.items(): self.u.reg_write(globals()['UC_X86_REG_'+key],value)
        self.u.emu_start(a,b,count=100000)
        assert self.u.reg_read(UC_X86_REG_EIP)==b


def lifecycle_cases(n):
    cases=[]
    def keep(name,**extra):
        cases.append({'case':name,'bond_after':n.bond(),'actor_owner_after':n.get(A+0x160),
                      'observations':n.events.copy(),**extra})
        n.events.clear()
    n.received()
    assert n.bond()==[OWNER,0,1,2,2]
    assert n.get(OCC+12)==42
    keep('fresh_SC139_stores_full_bond_and_applies')
    n.received()
    assert not n.events
    keep('duplicate_active_SC139_is_ignored')
    n.put(B,999,7,8,9,2)
    n.received()
    assert not n.events and n.bond()[0]==OWNER
    keep('different_SC139_while_active_is_ignored')
    n.put(B,OWNER,0,1,2,2)
    n.call(0x3935a520,U)
    assert n.bond()==[0,0xffffffff,0,0,1]
    assert [list(x)[0] for x in n.events]==['packet_sink','cancel_callback_endpoint','entity_detach_endpoint','action_endpoint','refresh_actor']
    assert n.events[1]['bond_owner_at_cancel']==0
    assert n.get(A+0x160)==0
    keep('local_unbond_sends_CD_then_clears_before_callback_cancel_detach_action_refresh')
    n.call(0x3935a520,U)
    assert not n.events
    keep('second_local_unbond_after_clear_sends_nothing')
    n.received()
    assert n.bond()[0]==OWNER and len(n.events)==1
    keep('fresh_same_bond_SC139_after_local_clear_reseats_without_SC13A')
    for actor,character,owner,label in [(ACTOR,42,OWNER,'matched'),(ACTOR,999,OWNER,'wrong_character'),
                                       (ACTOR,42,999,'wrong_owner'),(99,42,OWNER,'missing_actor')]:
        n.active()
        n.call(0x393b2bd0,0,actor,character,owner)
        assert (n.bond()[0]==0)==(actor==ACTOR)
        keep('SC13A_'+label,packet={'actor':actor,'character':character,'owner':owner})
    n.put(U+0x3b94,0,0xffffffff,0,0,1)
    n.call(0x393b2bd0,0,ACTOR,42,OWNER)
    assert not n.events
    keep('SC13A_after_local_clear_is_noop')
    n.owner_present=False
    n.received()
    assert n.bond()[0]==OWNER and 'queue_endpoint' in n.events[-1]
    keep('owner_absent_waits_for_callback_with_bond_retained')
    n.owner_present=True
    n.call(0x39362ca0,0,ACTOR,OWNER,0,0,1)
    assert any('apply_actor_bond_endpoint' in e for e in n.events)
    keep('ready_callback_reapplies_current_stored_bond')
    n.active()
    n.call(0x39362ca0,0,ACTOR,999,0,0,0)
    assert n.bond()[0]==0
    keep('failure_callback_clears_current_bond_even_if_callback_owner_differs')
    n.active();n.callback_actor_present=False
    n.call(0x39362ca0,0,ACTOR,OWNER,0,0,1)
    assert not n.events and n.bond()[0]==OWNER
    keep('callback_after_actor_registry_removal_does_nothing')
    n.callback_actor_present=True
    n.put(U+0x3b94,0,0xffffffff,0,0,1)
    n.call(0x39362ca0,0,ACTOR,OWNER,0,0,1)
    assert not any('apply_actor_bond_endpoint' in e for e in n.events)
    keep('stale_ready_callback_with_cleared_bond_cannot_restore_old_bond')
    for render,entity,label in [(False,True,'render_missing'),(True,False,'world_entity_missing')]:
        n.events.clear();n.render_present=render;n.entity_present=entity
        n.put(U+0x3b94,0,0xffffffff,0,0,1)
        n.received()
        assert n.bond()[0]==0 and n.events[0]['packet_sink']=='0xcd'
        keep('owner_found_'+label+'_requests_and_locally_detaches')
    n.render_present=n.entity_present=True
    for move,yaw in [(0,0),(1,0),(0,1),(1,1)]:
        n.active(); i,f=0x10060000,0x10061000
        n.u.mem_write(i+0xb8,bytes([move,yaw]));n.put(f-0x34,U)
        n.hooks[0x394fab30]=lambda:(n.events.append({'close_loot_endpoint':True}),n.ret())
        n.slice(0x39405f73,0x39405f98,EDI=i,EBP=f,ESP=0x201ff000)
        assert (n.bond()[0]==0)==bool(move or yaw)
        if move or yaw:
            assert 'close_loot_endpoint' in n.events[0] and n.events[1]['packet_sink']=='0xcd'
        keep('input_gate_loot_close_then_CD_then_detach',activity_b8=move,yaw_b9=yaw)
    # The earlier input block separately cancels casting, before closing loot/unbond.
    n.hooks[0x393966d0]=lambda:(n.events.append({'stop_primary_cast_endpoint':True}),n.ret())
    n.hooks[0x393967c0]=lambda:(n.events.append({'stop_secondary_cast_endpoint':True}),n.ret())
    n.hooks[0x39016c40]=lambda:(n.events.append({'input_state_reset_endpoint':True}),n.ret())
    for move,yaw in [(0,0),(1,0),(0,1),(1,1)]:
        for can_stop in [False,True]:
            n.hooks[0x39395d90]=lambda:n.ret(int(can_stop))
            n.active();n.u.mem_write(i+0xb8,bytes([move,yaw]));n.events.clear()
            n.slice(0x39405f25,0x39405f98,EDI=i,EBP=f,ESP=0x201ff000)
            pre=[]
            if move: pre=['stop_primary_cast_endpoint','stop_secondary_cast_endpoint','input_state_reset_endpoint']
            if yaw and can_stop:pre+=['stop_primary_cast_endpoint']
            names=[list(x)[0] for x in n.events]
            assert names[:len(pre)]==pre
            if move or yaw:assert names[len(pre):len(pre)+2]==['close_loot_endpoint','packet_sink']
            else:assert not names
            keep('input_full_exit_block_order',activity_b8=move,yaw_b9=yaw,turn_cast_can_stop=can_stop)
    return cases


def manager_cases(n):
    """Single-node normalized native callback trees. Search/erase are endpoint mocks."""
    cases=[]; m,h,node=0x10050000,0x10051000,0x10052000
    n.hooks[0x3900b030]=lambda:(n.put(n.arg(0),node),n.ret(n.arg(0),pop=8))
    def erase():
        out=n.arg(0)
        n.put(m+0x18,h)
        n.put(h,h,h,h)
        n.events.append({'erase_callback_tree_endpoint':True})
        n.ret(out,pop=8)
    n.hooks[0x3900a360]=erase
    n.hooks[0x3900b590]=lambda:None if n.u.reg_read(UC_X86_REG_ECX)==m else n.cancel()
    for entry,flag,name in [(0x3900a970,0,'owner_removed_notifies_failure'),
                             (0x3900b4e0,1,'owner_ready_notifies_success'),
                             (0x3900b590,0,'local_cancel_notifies_failure')]:
        n.put(m+0x18,h);n.put(h,node,node,node);n.u.mem_write(h+0x21,b'\1')
        n.put(node,h,h,h,OWNER,ACTOR,0,0,0x39362ca0)
        n.u.mem_write(node+0x21,b'\0')
        n.active();n.events.clear()
        args=(ACTOR,OWNER) if entry==0x3900b590 else (OWNER,)
        n.call(entry,m,*args)
        assert (n.bond()[0]==0)==(flag==0)
        cases.append({'case':name,'fixture':'one normalized callback node; lookup+erase mocked',
                      'observations':n.events.copy(),'bond_after':n.bond()})
    # There is no active subscription after ready dispatch erases its node.
    n.put(m+0x18,h);n.put(h,h,h,h);n.active();n.events.clear()
    n.call(0x3900a970,m,OWNER)
    assert n.bond()[0]==OWNER and not n.events
    cases.append({'case':'owner_removed_with_no_waiter_does_not_clear_active_unit_bond',
                  'observations':n.events.copy(),'bond_after':n.bond()})
    # Unit destructor cancels actor-keyed primary waiters without firing them.
    gm=0x3a1699c4
    n.put(gm+8,h);n.put(h,node,node,node)
    n.put(node,h,h,h,OWNER,ACTOR,0,0,0x39362ca0)
    n.u.mem_write(h+0x21,b'\1');n.u.mem_write(node+0x21,b'\0')
    def erase_primary():
        n.put(gm+8,h);n.put(h,h,h,h)
        n.events.append({'erase_actor_waiter_endpoint':True})
        n.ret(n.arg(0),pop=8)
    n.hooks[0x3900ad30]=erase_primary
    n.active();n.events.clear()
    n.slice(0x3936826d,0x39368282,ESI=U,EBP=0x10065000,ESP=0x201ff000)
    assert n.events==[{'erase_actor_waiter_endpoint':True}]
    cases.append({'case':'unit_destructor_slice_erases_actor_waiters_without_callback',
                  'observations':n.events.copy(),'boundary':'normalized primary callback tree; erase endpoint mocked; remaining unit/actor destruction excluded'})
    return cases


def entity_cases(n):
    raw=ENTITY.read_bytes();assert hashlib.sha256(raw).hexdigest()==ENTITY_SHA
    p=pefile.PE(data=raw);base=p.OPTIONAL_HEADER.ImageBase;im=p.get_memory_mapped_image()
    n.u.mem_map(base,(len(im)+4095)&~4095);n.u.mem_write(base,im)
    parent,child,pv,cv,pa,ca,vec,world=[0x10200000+i*0x1000 for i in range(8)]
    n.put(parent,pv);n.put(child,cv);n.put(parent+0xa0,pa);n.put(child+0xa0,ca)
    n.put(pa,vec,vec+4,vec+4,0,0);n.put(ca,0,0,0,0,parent);n.put(vec,child)
    n.put(pv+0x40,0x32013ff0);n.put(cv+0x44,0x32014040)
    n.put(cv+0x60,n.fn(lambda:n.ret(world)))
    captured=[]
    def local_tm():
        captured.append(list(struct.unpack('<12f',n.u.mem_read(n.arg(0),48))))
        n.events.append({'set_detached_local_matrix_endpoint':captured[-1],'flags':n.arg(1)})
        n.ret(pop=8)
    n.put(cv+0x5c,n.fn(local_tm))
    n.put(cv,n.fn(lambda:n.ret(ACTOR)))
    def event():
        n.events.append({'entity_event_sink':n.get(n.arg(0)),'receiver':n.u.reg_read(UC_X86_REG_ECX)})
        n.ret(pop=4)
    n.put(pv+0xb8,n.fn(event));n.put(cv+0xb8,n.fn(event))
    def remove_child():
        assert n.get(n.arg(1))==child
        n.put(pa+4,vec)
        n.ret()
    n.hooks[0x32016d00]=remove_child
    matrix=[0,-1,0,11,1,0,0,22,0,0,1,33]
    n.u.mem_write(world,struct.pack('<12f',*matrix))
    n.events.clear();n.call(0x32013ff0,parent,1)
    assert n.get(ca+0x10)==0 and n.get(pa+4)==vec and captured==[matrix]
    cases=[{'case':'native_DetachAll1_keeps_child_world_matrix_and_clears_parent',
            'observations':n.events.copy(),'child_parent_after':n.get(ca+0x10),
            'mock_boundaries':'world-matrix getter/local-matrix setter, vector erase and entity-event sinks; detach+loop instructions original'}]
    # Execute the actual shutdown attachment slice, including DetachAll and free.
    n.put(pa,vec,vec+4,vec+4,0,0);n.put(ca,0,0,0,0,parent)
    n.events.clear();captured.clear()
    n.hooks[0x320061b0]=lambda:(n.events.append({'free_attachment_storage_endpoint':n.arg(0)}),n.ret())
    n.put(0x320c3074,n.fn(lambda:n.ret(pop=4)))
    n.slice(0x320153ca,0x32015426,ESI=parent,EBX=0,EBP=0x10065000,ESP=0x201ff000)
    assert n.get(ca+0x10)==0 and captured==[matrix]
    assert [x['free_attachment_storage_endpoint'] for x in n.events if 'free_attachment_storage_endpoint' in x]==[vec,pa]
    cases.append({'case':'actual_ShutDown_attachment_slice_detaches_child_before_free',
                  'observations':n.events.copy(),'child_parent_after':n.get(ca+0x10),
                  'mock_boundaries':'same detach endpoints plus allocator/destructor sinks; unrelated earlier shutdown systems not executed'})
    # Native SetWorldTM computes inverse(parent_world) * requested_world.
    parent_world=0x10208000
    n.u.mem_write(parent_world,struct.pack('<12f',0,-2,0,10,2,0,0,20,0,0,2,30))
    n.put(cv+0x50,0x32016b20)
    n.put(pv+0x60,n.fn(lambda:n.ret(parent_world)))
    n.put(ca+0x10,parent);n.events.clear();captured.clear()
    n.call(0x32010420,child,world,0)
    expected=[.5,0,0,1,0,.5,0,-.5,0,0,.5,1.5]
    assert len(captured)==1 and all(abs(x-y)<1e-5 for x,y in zip(captured[0],expected)),captured
    cases.append({'case':'native_SetWorldTM_converts_unit_world_scale_to_inverse_parent_local_scale',
                  'parent_world':[0,-2,0,10,2,0,0,20,0,0,2,30],
                  'requested_child_world':matrix,'child_local_handoff':captured[0],
                  'mock_boundaries':'parent-world getter/local-TM setter endpoints; original parent lookup, matrix inversion/product execute; no per-frame engine transform propagation'})
    return cases


def rebuild_and_phase_cases(n):
    cases=[]
    # Actor-view rebuild's original bond subroutine (earlier actor allocation is excluded).
    ready=True
    n.endpoint(RV+0x7c,lambda:n.ret(int(not ready)))
    def template_id():
        n.put(n.arg(0),1236)
        n.ret(n.arg(0),pop=4)
    n.endpoint(DV+4,template_id)
    n.hooks[0x3983c0a0]=lambda:n.ret(0x10066000)
    for present,loading,has_bond_part,label in [(False,False,True,'owner_absent'),
                (True,True,True,'owner_loading'),(True,False,True,'owner_ready'),
                (True,False,False,'current_phase_has_no_template_part_0x3b')]:
        n.owner_present=present;ready=not loading
        n.hooks[0x3983c0a0]=lambda:n.ret(0x10066000 if has_bond_part else 0)
        n.active();n.events.clear()
        n.slice(0x39364ec8,0x39364f52,ESI=U,EBP=0x10065000,ESP=0x201ff000)
        assert (n.bond()[0]==0)==(not has_bond_part)
        cases.append({'case':'actor_render_rebuild_'+label,'bond_after':n.bond(),'observations':n.events.copy(),
                      'boundary':'actor allocation excluded; owner readiness/template-part-0x3b lookup are endpoints'})
    n.owner_present=True
    # Phase replacement of the owner's render wrapper. No live renderer/layout construction.
    group,new_r,rv2,old_id=0x10230000,0x10231000,0x10232000,1951
    n.put(group+0x10,0x10233000);n.u.mem_write(0x10233000,b'prefab://fixture/current_phase\0')
    n.put(D+0x220,old_id);n.put(D+0x21c,R);n.put(D+0x88,0)
    n.put(R+4,rv2);n.put(new_r+4,rv2)
    n.endpoint(rv2+4,lambda:(n.events.append({'render_addref_endpoint':True}),n.ret()))
    n.endpoint(rv2+8,lambda:(n.events.append({'render_release_endpoint':True}),n.ret()))
    n.endpoint(DV+0x1c,lambda:n.ret())
    for iat in [0x38ff6f3c,0x38ff6e30]:n.put(iat,n.fn(lambda:n.ret()))
    n.put(0x38ff6c60,n.fn(lambda:n.ret(1)))
    n.hooks[0x390115a0]=lambda:n.ret(pop=16)
    n.hooks[0x390116d0]=lambda:n.ret()
    n.hooks[0x3963b250]=lambda:n.ret(group)
    n.hooks[0x393b0940]=lambda:(n.events.append({'phase_render_factory_endpoint':True}),n.ret(new_r))
    n.hooks[0x393a6670]=lambda:n.ret()
    n.hooks[0x393a4360]=lambda:n.ret()
    n.hooks[0x393a3660]=lambda:n.ret(pop=4)
    n.hooks[0x393a2e10]=lambda:n.ret(pop=4)
    n.hooks[0x393a3070]=lambda:n.ret(pop=4)
    n.active();n.events.clear()
    n.call(0x393a6c60,D,22863,0,0,0)
    assert n.get(D+0x220)==22863 and n.get(D+0x21c)==new_r
    assert n.bond()[0]==OWNER and not any('apply_actor_bond_endpoint' in e or 'entity_detach_endpoint' in e for e in n.events)
    cases.append({'case':'phase_render_wrapper_replacement_does_not_clear_or_reapply_active_seat',
                  'observations':n.events.copy(),'unit_bond_after':n.bond(),
                  'boundary':'native phase method and wrapper swap execute; template/render factory, render refcount/destruction, visual refresh endpoints mocked; no claim about resulting live layout hierarchy'})
    return cases


def animation_cases(n):
    mgr,graph,vt,row,out,act=[0x10210000+i*0x1000 for i in range(6)]
    n.put(mgr+0xc,graph);n.put(graph,vt);n.put(row,6,0x10218000)
    n.u.mem_write(0x10218000,b'fist_ac_sit_down\0')
    n.put(act+0x38,U);n.put(mgr+4,act)
    n.hooks[0x396f6190]=lambda:n.ret(row)
    n.hooks.pop(0x39155ed0,None)
    n.put(vt+4,n.fn(lambda:(n.events.append({'graph_input_sink':[n.arg(i) for i in range(3)]}),n.ret(pop=12))))
    n.hooks[0x39155310]=lambda:n.ret()
    cases=[]
    n.events.clear();n.call(0x39155ed0,mgr,19,1,0)
    assert n.get(mgr+0x58)==19 and not n.events
    cases.append({'case':'action19_sets_base_slot6_without_erasing_performance_queue',
                  'base_slot6':n.get(mgr+0x58),'observations':n.events.copy()})
    n.put(mgr+0x34,0);n.put(mgr+0x64,0)
    n.call(0x391555d0,mgr)
    assert n.events and n.get(mgr+0x64)==19
    cases.append({'case':'no_queued_animation_base_selector_publishes19',
                  'selected_action':n.get(mgr+0x64),'observations':n.events.copy()})
    n.events.clear();n.put(mgr+0x34,1)
    n.call(0x391555d0,mgr)
    assert n.get(mgr+0x64)==1 and n.get(mgr+0x58)==19 and not n.events
    cases.append({'case':'queued_animation_suppresses_base_publish_but_retains19',
                  'selected_action':n.get(mgr+0x64),'base_slot6':n.get(mgr+0x58),'observations':n.events.copy()})
    n.put(mgr+0x34,0);n.call(0x391555d0,mgr)
    assert n.get(mgr+0x64)==19 and n.events
    cases.append({'case':'queue_cleared_base_selector_returns_to19',
                  'selected_action':n.get(mgr+0x64),'observations':n.events.copy()})
    n.events.clear();n.call(0x39155ed0,mgr,19,0,0)
    assert n.get(mgr+0x58)==0
    cases.append({'case':'seat_action_disable_clears_only_slot6',
                  'base_slot6':n.get(mgr+0x58),'observations':n.events.copy()})
    # The queued-animation publication slice uses category6/loop=true from catalog.json.
    # Its variant resolver is an endpoint, so this does not execute buff ingestion.
    head,node,definition,clip=0x10219000,0x1021a000,0x1021b000,0x1021c000
    n.put(mgr+0x58,19);n.put(mgr+0x30,head,1);n.put(head,node,node)
    n.put(node,head,head);n.put(node+8+0x1c,definition,0)
    n.put(definition,6);n.u.mem_write(definition+0x20,b'\1')
    n.u.mem_write(mgr+0x10,struct.pack('<HH',5,6))
    n.put(vt+0x24,n.fn(lambda:n.ret(pop=8)))
    n.hooks[0x39154ad0]=lambda:n.ret(clip)
    n.hooks[0x391547b0]=lambda:n.ret(pop=4)
    for ident,name in [(584,'music_co_sk_sit_down_piano_cast'),(616,'music_co_sk_sit_down_drum_cast')]:
        n.u.mem_write(clip,name.encode()+b'\0');n.u.mem_write(mgr+0x68,b'\1')
        n.put(0x10065000+8,struct.unpack('<I',struct.pack('<f',1))[0]);n.events.clear()
        n.slice(0x39156c15,0x39156d12,ESI=mgr,EBP=0x10065000,ESP=0x201ff000)
        assert n.events[0]['graph_input_sink']==[5,clip,0] and n.get(mgr+0x58)==19
        cases.append({'case':'queued_category6_loop_publishes_performance_Action_over_retained_seat',
                      'anims_id':ident,'clip':name,'base_slot6':n.get(mgr+0x58),'observations':n.events.copy(),
                      'boundary':'normalized animation queue and category6/loop metadata fixture; clip-variant resolver and graph sink mocked; buff ingest not executed'})
    return cases


if __name__=='__main__':
    n=Probe()
    cases=lifecycle_cases(n)+manager_cases(n)+entity_cases(n)+rebuild_and_phase_cases(n)+animation_cases(n)
    result={'game_sha256':SHA,'entity_dll':str(ENTITY),'entity_sha256':ENTITY_SHA,'runtime_sha256':RUNTIME_SHA,
            'limits':'Original x86 attachment gates, bond store/clear, packet constructor, callback dispatch, callback apply/fail, native entity detach/shutdown slice, input gate and action-slot selection execute. Actor attach/helper/render, actor registry/owner lookup, networking, graph output, profiler, normalized callback tree search/erase, entity TM/vector/event/allocator endpoints are fixtures. No live game, renderer, server, device, full phase loader, full buff pipeline, full actor destruction or relog execution.',
            'cases':cases}
    (OUT/'lifecycle-cases.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(len(cases),'original-instruction lifecycle cases passed; lifecycle-cases.json')
