"""Read-only original 1.2 mode mutation/resolution, isolated Unicorn fixtures.

Outputs retained evidence under tools/evidence/user_music_actions. The original enable/disable, node helper,
resolver, map lookup and quest-enter wrapper execute; no production code changes.
"""
from pathlib import Path
import hashlib
import itertools
import json
import struct
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from user_music_lifecycle_oracle import Native, SHA, RUNTIME_DATA, RUNTIME_SHA
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_ESI

OUT = Path(__file__).parent / "evidence" / "user_music_actions"
n = Native()
MANAGER, HEAD, MAPS, EVENT = 0x3A59F1C0, 0x10081000, 0x10082000, 0x10083000
LUA, VT, LUA_RETURN = 0x10088000, 0x10089000, 0x30000600
KEY = 0x1200
effects, frees, visits, entries = [], [], [], []

def effect():
    effects.append({'id': n.arg(0), 'payload': n.arg(1)})
    n.ret(pop=8)

def free():
    frees.append(n.arg(0))
    n.ret()

def mutation_entry():
    entries.append([n.arg(0), n.arg(1)])

def active_visit():
    node = n.get(n.u.reg_read(UC_X86_REG_ESI) + 4)
    visits.append(n.get(node + 8))

n.hooks.update({0x39202DB0: effect, 0x3950A1F0: free,
                0x393FE030: mutation_entry, 0x393FC2A0: active_visit,
                0x393FC348: lambda: visits.append(0),
                LUA_RETURN: lambda: n.ret()})
n.put(0x39D1F970, 1)  # initialized event dispatcher; getter remains original
n.put(LUA, VT)
n.put(VT + 0x2C, LUA_RETURN)

def reset():
    n.put(MANAGER + 0x10C, HEAD, 0)
    n.put(HEAD, HEAD, HEAD, 0xFFFFFFFF)
    n.heap = 0x10100000
    effects.clear(); frees.clear(); entries.clear(); visits.clear()
    n.put(0x39D29288, 0)
    maps({i: KEY for i in range(5)})

def maps(keys):
    for i in range(5):
        head, node, action = 0x10084000 + i * 0x100, 0x10084040 + i * 0x100, 0x10086000 + i * 0x200
        n.u.mem_write(head, bytes(0x80))
        n.u.mem_write(action, bytes(0x180))
        n.put(MAPS + i * 16, 0, head, int(i in keys), 0)
        n.put(head, head, head, head)
        n.u.mem_write(head + 0x15, b'\1')
        if i in keys:
            n.put(head, node, node, node)
            n.put(node, head, head, head, keys[i], action)

def state():
    order, nodes = [], []
    node, previous = n.get(HEAD), HEAD
    while node != HEAD:
        assert len(order) < 10
        assert n.get(node + 4) == previous
        order.append(n.get(node + 8)); nodes.append(node)
        previous, node = node, n.get(node)
    assert n.get(HEAD + 4) == previous
    assert len(order) == n.get(MANAGER + 0x110)
    assert len(set(order)) == len(order)
    return order, nodes

def resolve(key=KEY):
    visits.clear()
    n.put(EVENT + 0x14, key)
    action = n.call(0x393FC270, MANAGER, MAPS, EVENT)
    mode = None if not action else (action - 0x10086000) // 0x200
    return {'mode': mode, 'visited': list(visits), 'event_key_after': n.get(EVENT + 0x14),
            'action_flags': n.get(action + 0x10C) if action else None}

def mutate(index, enabled):
    before_effects, before_frees, before_heap = len(effects), len(frees), n.heap
    n.call(0x393FE030, MANAGER, index, enabled)
    order, nodes = state()
    return {'call': [index, enabled], 'oldest_to_newest': order, 'nodes': nodes,
            'effect_ids': [x['id'] for x in effects[before_effects:]],
            'freed': len(frees) - before_frees, 'allocated_bytes': n.heap - before_heap,
            'resolve_collision': resolve()}

cases = []
for permutation in itertools.permutations([1, 2, 4]):
    reset(); steps = []
    for index in permutation:
        step = mutate(index, 1); steps.append(step)
        assert step['resolve_collision']['mode'] == index
    assert state()[0] == list(permutation)
    assert all(s['effect_ids'] == [43] for s in steps)
    cases.append({'case': 'enable_permutation', 'order': permutation, 'steps': steps})

reset()
steps = [mutate(i, 1) for i in [1, 2, 4]]
tail_node = state()[1][-1]
step = mutate(4, 1); steps.append(step)
assert step['nodes'][-1] == tail_node and step['allocated_bytes'] == step['freed'] == 0 and step['effect_ids'] == []
step = mutate(1, 1); steps.append(step)
assert step['oldest_to_newest'] == [2, 4, 1] and step['freed'] == 1 and step['effect_ids'] == [43]
step = mutate(2, 1); steps.append(step)
assert step['oldest_to_newest'] == [4, 1, 2] and step['freed'] == 1
cases.append({'case': 'duplicates_and_reenable_promotes', 'steps': steps})

reset()
steps = [mutate(i, 1) for i in [1, 2, 4]]
for index, expected in [(2, [1, 4]), (2, [1, 4]), (4, [1]), (1, [])]:
    step = mutate(index, 0); steps.append(step)
    assert step['oldest_to_newest'] == expected
assert steps[4]['effect_ids'] == [] and steps[4]['freed'] == 0
assert steps[-1]['resolve_collision']['mode'] == 0
cases.append({'case': 'disable_preserves_other_modes_and_restores_previous', 'steps': steps})

reset()
steps = []
for index, flag, expected in [(1, 2, [1]), (2, 255, [1, 2]), (1, 256, [2]), (4, 257, [2, 4]),
                              (5, 1, [2, 4]), (0xFFFFFFFF, 1, [2, 4]), (0, 1, [2, 4, 0]), (0, 0, [2, 4])]:
    step = mutate(index, flag); steps.append(step)
    assert step['oldest_to_newest'] == expected
assert steps[4]['effect_ids'] == steps[5]['effect_ids'] == []
cases.append({'case': 'range_and_low_byte_boolean', 'steps': steps})

reset()
for index in [1, 2, 4]: mutate(index, 1)
lookups = []
for keys, expected, visited in [({0: KEY, 1: KEY, 2: KEY}, 2, [4, 2]),
                                ({0: KEY, 1: KEY}, 1, [4, 2, 1]),
                                ({0: KEY}, 0, [4, 2, 1, 0]),
                                ({}, None, [4, 2, 1, 0])]:
    maps(keys); result = resolve()
    assert result['mode'] == expected and result['visited'] == visited
    lookups.append(result)
cases.append({'case': 'nested_modes_all_participate_then_normal', 'lookups': lookups})

reset()
for index in [1, 2, 4]: mutate(index, 1)
maps({0: KEY | 0x44, 1: KEY | 0x44, 2: KEY | 0x44, 4: KEY})
result = resolve(KEY | 0x44)
assert result == {'mode': 4, 'visited': [4], 'event_key_after': KEY, 'action_flags': 0x44}
cases.append({'case': 'newest_mode_modifier_fallback_precedes_older_exact', 'result': result})

reset()
steps = [mutate(i, 1) for i in [2, 1]]
for k in range(2):
    before = len(effects)
    n.call(0x3949B190, 0, LUA)  # actual Lua EnterQuestDirectingMode and native quest enter
    assert state()[0] == [2, 1, 4] and n.get(0x39D29288) & 255 == 1
    steps.append({'producer': '3949b190 EnterQuestDirectingMode', 'oldest_to_newest': state()[0],
                  'effect_ids': [x['id'] for x in effects[before:]], 'resolve_collision': resolve()})
    if k == 0:
        step = mutate(1, 1); steps.append(step)
        assert step['oldest_to_newest'] == [2, 4, 1]
assert steps[2]['effect_ids'] == [12, 43] and steps[-1]['effect_ids'] == [43]
cases.append({'case': 'quest_enter_and_modebar_reassert_can_promote_each_other', 'steps': steps})

reset()
runtime = RUNTIME_DATA.read_bytes()
assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
for address in [0x39D1FB9C, 0x39AC62BC]:
    n.put(address, struct.unpack_from('<I', runtime, address-0x39AC3000)[0])
BAR, VECTOR = 0x100B0000, 0x100C0000
n.put(0x3A1744F0, VECTOR, VECTOR+4, VECTOR+0x1000)
n.put(VECTOR, 20848)
n.put(0x39B02294, 0)
n.hooks[0x39204500] = lambda: n.ret()  # unrelated mount/base population boundary
ui_events = []
def ui_event():
    ui_events.append({'event': n.arg(0), 'active_modes': state()[0], 'mode_count': n.get(BAR+0x13B0)})
    n.ret()
n.hooks[0x39424220] = ui_event
for index in [2, 4]: mutate(index, 1)
steps = []
for name, before, expected in [('first_bar_refresh', None, [2, 4, 1]),
                               ('unchanged_bar_refresh_already_newest', None, [2, 4, 1]),
                               ('quest_promoted_then_unchanged_bar_refresh', 4, [2, 4, 1]),
                               ('empty_bar_refresh', 0, [2, 4])]:
    if before == 4: mutate(4, 1)
    if before == 0: n.put(0x3A1744F4, VECTOR)
    start_effects, start_ui = len(effects), len(ui_events)
    n.call(0x392045F0, BAR)
    assert state()[0] == expected
    assert len(ui_events) == start_ui+1 and ui_events[-1]['event'] == 95
    steps.append({'case': name, 'oldest_to_newest': state()[0],
                  'effect_ids': [x['id'] for x in effects[start_effects:]],
                  'ui_event': ui_events[-1], 'resolve_collision': resolve()})
assert steps[0]['effect_ids'] == [43] and steps[1]['effect_ids'] == [] and steps[2]['effect_ids'] == [43]
cases.append({'case': 'original_full_modebar_rebuild_reasserts_mode1_and_empty_removes_only_mode1', 'steps': steps})

reset()
BUILD_SYSTEM, BUILD_SYSTEM_VT, BUILD_OBJECT, BUILD_VT, BUILD_CALLER = 0x1008A000, 0x1008B000, 0x1008C000, 0x1008D000, 0x1008E000
n.put(n.get(0x3A5BB1D8)+0x14, BUILD_SYSTEM)
n.put(BUILD_SYSTEM, BUILD_SYSTEM_VT)
n.put(BUILD_SYSTEM_VT+0x10, 0x30000610)
n.hooks[0x30000610] = lambda: n.ret(0, pop=4)  # clock service
n.hooks[0x393A6D40] = lambda: n.ret(BUILD_OBJECT)  # current builder-object lookup
n.put(BUILD_OBJECT, BUILD_VT)
n.put(BUILD_OBJECT+0xC, 2)
n.put(BUILD_VT, 0x30000620, 0x30000620)
n.hooks[0x30000620] = lambda: n.ret()  # builder object's operation leaf
for index in [4, 1]: mutate(index, 1)
steps = []
for name, expected in [('build_state_2', [4, 1, 2]), ('build_reassert_after_modebar', [4, 1, 2]),
                        ('build_state_other_disables_only_2', [4, 1])]:
    if name == 'build_reassert_after_modebar': mutate(1, 1)
    if name == 'build_state_other_disables_only_2': n.put(BUILD_OBJECT+0xC, 0)
    start = len(effects)
    n.call(0x39017220, BUILD_CALLER, 1)
    assert state()[0] == expected
    steps.append({'case': name, 'oldest_to_newest': state()[0],
                  'effect_ids': [x['id'] for x in effects[start:]], 'resolve_collision': resolve()})
cases.append({'case': 'original_building_producer_preserves_quest_and_modebar', 'steps': steps})

catalog = [{'index': i, 'table_address': hex(0x39AC9020 + 4 * i),
            'string_address': hex(n.get(0x39AC9020 + 4 * i)),
            'name': n.string(n.get(0x39AC9020 + 4 * i)).decode()} for i in range(5)]
sources = []
cs = Cs(CS_ARCH_X86, CS_MODE_32)
for start, end in [(0x393FE030, 0x393FE0FB), (0x393FABC0, 0x393FABF3),
                   (0x393FC270, 0x393FC3F3), (0x393FA7F0, 0x393FA852),
                   (0x3949B190, 0x3949B1E4), (0x3921ACF0, 0x3921AD5B),
                   (0x394A90C8, 0x394A914A), (0x393FCE55, 0x393FCE94),
                   (0x393FF2C0, 0x393FF2E9), (0x39202FB0, 0x3920301A),
                   (0x392045F0, 0x39204710), (0x39017220, 0x390172EA)]:
    sources.append(f'\nSOURCE {start:08x}..{end:08x}')
    for ins in cs.disasm(bytes(n.u.mem_read(start, end-start)), start):
        sources.append(f'{ins.address:08x}: {ins.mnemonic} {ins.op_str}')
(OUT / 'input-mode-executed-source.txt').write_text('\n'.join(sources), encoding='utf-8')
result = {'game_sha256': SHA, 'catalog': catalog, 'cases': cases,
          'executed': ['393fe030', '393fabc0', '393fc270', '393fa7f0', '39202fb0', '3949b190', '3921acf0', '392045f0', '392044a0', '39017220'],
          'substituted': ['3950a1b0 allocator', '3950a1f0 free observer', '39202db0 event dispatch sink',
                          'fixture Lua return virtual endpoint', '39204500 unrelated mount/base population',
                          '39424220 Lua/UI event sink', '393a6d40 current-builder lookup',
                          'fixture clock service and builder-operation virtual endpoints'],
          'not_claimed': 'No live client, server, OS input or rendering proof. Synthetic lists and binding maps.'}
(OUT / 'input-mode-executed-cases.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
print(json.dumps({'passed_cases': len(cases), 'enable_permutations': 6, 'catalog': catalog,
                  'output': str(OUT / 'input-mode-executed-cases.json')}))
