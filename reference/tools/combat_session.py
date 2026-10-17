"""reference/tools/combat_session.py -- the combat captures of reference/ingame/combat_* (own client, in world, Refone).

  python -I tools/combat_session.py melee death triple_slash kick return

Needs the client in world near an Angry Rat (see README / notes.md in the folders). Every step is one burst of screenshots
around a key press (combat.py), retried when the burst stalled."""
import json, math, os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import combat as k

c = k.c
m = k.Mcp()


def setup():
    c.lua_file(k.PID, 'lib.lua', 'ui')
    c.lua_file(k.PID, 'lib.lua', 'system')
    c.lua(k.PID, "System.SetCVar('cl_fov', 75) return 1", 'system')   # wider lens: with the default camera (pitch 0, 4 m behind, 1.73 m up) the ground at the player's depth is below the screen
    time.sleep(1.0)


def wait_rat(max_s=90):
    t0 = time.time()
    while time.time() - t0 < max_s:
        l, pl = k.near(m, 'Rat')
        if l and l[0][0] < 14:
            return l[0][0]
        time.sleep(2)
    return None


def dead_or_gone(max_s=25):
    t0 = time.time()
    while time.time() - t0 < max_s:
        d, t = k.tdist(m)
        if d is None:
            return True
        time.sleep(0.4)
    return False


def step_melee_death():
    shutil.rmtree(os.path.join(c.REF, 'ingame', 'combat_death'), ignore_errors=True)
    for attempt in range(3):
        wait_rat()
        meta = k.capture(m, 'melee', 2, '3', pre=0.3, dur=3.6, mob='Rat',
                         note='basic melee auto attack (Melee Attack, skill 2, key 3; it repeats about once a second until the target dies) on an Angry Rat; cl_fov 75')
        ok = k.quality(meta, min_frames=8)
        print('melee quality', ok)
        if ok:
            break
        dead_or_gone(); time.sleep(8)
    # same fight: wait for the last hit and capture the death
    out = os.path.join(c.REF, 'ingame', 'combat_death')
    hp_log = []; t0 = time.time(); h = None
    while time.time() - t0 < 25:
        u = k.unit_summary(m); h = u['target']['health']; hp_log.append((round(time.time() - t0, 2), h))
        if h is None or h <= 45:
            break
        time.sleep(0.08)
    if h is None:
        print('death: the rat was already dead; will retry with a new fight')
        return False
    lp = k.log_size(); tw = time.strftime('%Y-%m-%d %H:%M:%S')
    before = k.full_state(m, 'Rat')
    log = k.burst(out, lambda: 'burst started with the rat at %s HP' % h, pre=0.0, dur=5.5)
    after = k.full_state(m, 'Rat'); ua = k.unit_summary(m)
    c.save_json(os.path.join(out, 'camera.json'), {'before_burst': before, 'after_burst': after})
    c.save_json(os.path.join(out, 'frames.json'), {'name': 'death', 'skill_id': 2, 'captured': tw,
        'note': 'Angry Rat killed by the repeating Melee Attack (skill 2); time 0 = start of the burst (the rat had %s HP left), there is no key press in this burst; cl_fov 75' % h,
        'hp_poll_s_hp': hp_log[-8:], 'units_after': ua, 'camera_before': before, 'camera_after': after, 'burst': log, 'server_log': k.log_since(lp)})
    print('death frames', len(log['frames']), ua)
    return True


def step_skill(name, skill_id, key, shift, dur, note):
    for attempt in range(3):
        dead_or_gone(); wait_rat()
        meta = k.capture(m, name, skill_id, key, shift=shift, pre=0.3, dur=dur, mob='Rat', note=note + '; cl_fov 75')
        ok = k.quality(meta, min_frames=7)
        print(name, 'quality', ok, meta['prepare'])
        if ok and meta['units_after']['target'].get('health') != meta['units_before']['target'].get('health'):
            return True
        time.sleep(5)
    return False


if __name__ == '__main__':
    what = sys.argv[1:] or ['melee', 'death', 'triple_slash', 'kick']
    setup()
    if 'melee' in what or 'death' in what:
        step_melee_death()
    if 'triple_slash' in what:
        step_skill('triple_slash', 18132, '1', False, 3.6, 'Triple Slash (Battlerage, skill 18132, action bar slot 1, key 1) on an Angry Rat')
    if 'kick' in what:
        step_skill('kick', 14495, '2', True, 3.0, 'Kick (skill 14495, action bar slot 14, SHIFT-2) on an Angry Rat')
    m.close()
