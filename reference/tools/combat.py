"""reference/tools/combat.py -- helpers for the combat captures (persistent MCP session, key presses, timestamped bursts).

  python -I - <<'X'
  import sys; sys.path.insert(0, 'tools'); import combat as k
  m = k.Mcp(); print(m.call('client_unit_state', {'pid': k.PID, 'target': 'player'}))
  X
"""
import json, os, subprocess, sys, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture as c

PID = int(open(c.PIDFILE).read())


class Mcp:
    """One long-lived `aaemu-clientdriver mcp` process, so key presses cost milliseconds instead of a process start."""
    def __init__(self):
        self.p = subprocess.Popen([c.DRIVER, 'mcp', '--dll', os.environ['AADRIVER_DLL']], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.n = 1
        self._send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'ref', 'version': '1'}}})
        self._recv()
        self._send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})

    def _send(self, m):
        self.p.stdin.write((json.dumps(m) + chr(10)).encode()); self.p.stdin.flush()

    def _recv(self):
        return json.loads(self.p.stdout.readline().decode())

    def call(self, tool, args):
        self.n += 1
        self._send({'jsonrpc': '2.0', 'id': self.n, 'method': 'tools/call', 'params': {'name': tool, 'arguments': args}})
        r = self._recv()
        res = r.get('result', {})
        sc = res.get('structuredContent')
        if sc is not None:
            return sc
        txt = res.get('content', [{}])[0].get('text', '')
        try:
            return json.loads(txt)
        except Exception:
            return txt

    def key(self, key, ms=120, shift=False):
        a = {'pid': PID, 'key': key, 'duration_ms': ms}
        if shift:
            a['modifiers'] = ['shift']
        return self.call('client_input', a)

    def action(self, name):
        return self.call('client_action', {'pid': PID, 'action': name})

    def close(self):
        try: self.p.kill()
        except Exception: pass


def lua(code, target='ui'):
    return c.lua(PID, code, target).strip()


def cam_state():
    """View camera + player entity + distance, one System-state call."""
    code = """
local cp = System.GetViewCameraPos(); local ca = System.GetViewCameraAngles()
local me = X2Unit:UnitName('player')
local out = { cam = cp, ang = ca, fov = System.GetViewCameraFov(), tod = System.GetCVar('e_time_of_day') }
for _, e in pairs(System.GetEntities() or {}) do
  if e:GetName() == me then out.player = e:GetWorldPos(); out.pyaw = e:GetAngles().z end
end
return out
"""
    code = code.replace('X2Unit:UnitName(\'player\')', "'" + os.environ.get('AA_CHAR','Refone') + "'")
    r = c.lua(PID, 'local o = (function() ' + code + ' end)() return __ref.json(o)', 'system').strip()
    return json.loads(r)


import math


def near(m, name, limit=30):
    """Entities whose name contains `name`, nearest first: [(dist_xy, entity_dict)], plus the player position."""
    pl = cam_state()['player']
    d = m.call('client_entities', {'pid': PID, 'name': name, 'limit': limit})
    out = []
    for v in (d.get('entities') or {}).values():
        p = v['position']
        out.append((math.hypot(p['x'] - pl['x'], p['y'] - pl['y']), v))
    out.sort(key=lambda t: t[0])
    return out, pl


def yaw_to(pl, p):
    """Cry yaw (degrees) that faces from pl to p (0 = +Y, +90 = -X)."""
    return math.degrees(math.atan2(-(p['x'] - pl['x']), p['y'] - pl['y']))


def angdiff(a, b):
    return (a - b + 180) % 360 - 180


def turn_to(m, deg, tol=2.0, tries=8):
    """Turn the character (and its camera) to Cry yaw `deg` with the A/D keys (about 55 deg/s)."""
    for _ in range(tries):
        s = cam_state()
        diff = angdiff(deg, math.degrees(s['pyaw']))
        if abs(diff) <= tol:
            return diff
        m.key('a' if diff > 0 else 'd', max(30, int(abs(diff) / 55.0 * 1000 * 0.9)))
        time.sleep(0.5 + abs(diff) / 55.0)
    return angdiff(deg, math.degrees(cam_state()['pyaw']))


def burst(out_dir, trigger, pre=0.6, dur=4.0, tag='', start_index=1):
    """Screenshots as fast as the hook allows (~0.3 s each, strictly one at a time, driver pipe is single instance)
    around `trigger()`: the first frame boundary after `pre` seconds runs trigger() (a key press), the loop runs on
    until `dur` seconds after the key press. Writes <out_dir>/real_<NN>.png and returns the frame log: for each frame the
    time the request was made and the time the driver reported the file written, in seconds after the key press
    (negative = before). The picture is the back buffer of the first presented frame after the request, so it lies
    between the two (close to t_req_s + ~0.05 s)."""
    os.makedirs(out_dir, exist_ok=True)
    frames = []
    t_start = time.time()
    t_key = t_key_done = None
    result = None
    i = start_index
    while True:
        now = time.time()
        if t_key is None and now - t_start >= pre:
            t_key = time.time()
            result = trigger()
            t_key_done = time.time()
        if t_key is not None and time.time() - t_key >= dur:
            break
        path = os.path.join(out_dir, 'real_%02d.png' % i)
        t0 = time.time()
        r = subprocess.run([c.DRIVER, 'screenshot', '--pid', str(PID), path], capture_output=True)
        t1 = time.time()
        if r.returncode == 0:
            frames.append({'file': os.path.basename(path), 't_req': t0, 't_done': t1})
            i += 1
    log = [{'file': f['file'], 't_req_s': round(f['t_req'] - t_key, 3), 't_done_s': round(f['t_done'] - t_key, 3)} for f in frames]
    return {'key_press_sent_at_s': 0.0, 'key_press_returned_s': round(t_key_done - t_key, 3), 'trigger_result': result, 'frames': log}


import threading


SERVER_LOG = os.path.join(c.SERVER, 'rs-game', 'logs', 'Server.log')


def log_size():
    return os.path.getsize(SERVER_LOG)


def log_since(pos):
    with open(SERVER_LOG, 'rb') as f:
        f.seek(pos)
        txt = f.read().decode('utf-8', 'replace')
    keep = ('StartSkill', 'Skill', 'skill', 'Damage', 'damage', 'died', 'Died', 'death', 'Death', 'killed', 'Kill', 'Teleport', 'teleport', 'Recall')
    return [l[:220] for l in txt.splitlines() if any(k in l for k in keep) and 'Active Instances' not in l and 'AI tick' not in l]


TARGET_JSON = r'''
local function f(...) return (select(1, ...)) end
return __ref.json({ target = X2Unit:UnitName('target'), hp = X2Unit:UnitHealth('target'), maxhp = X2Unit:UnitMaxHealth('target'),
  php = X2Unit:UnitHealth('player'), pmp = X2Unit:UnitMana('player') })
'''


def unit_summary(m):
    t = m.call('client_unit_state', {'pid': PID, 'target': 'target'})
    p = m.call('client_unit_state', {'pid': PID, 'target': 'player'})
    return {'target': {'present': t.get('present'), 'name': t.get('name'), 'health': t.get('health'), 'max_health': t.get('max_health'), 'distance_m': t.get('distance')},
            'player': {'health': p.get('health'), 'max_health': p.get('max_health')}}


def prepare(m, mob='Fox', tol=2.0, camera='right_camera'):
    """Make sure a live hostile `mob` is targeted, the character faces it, and the camera is snapped side-on."""
    t = m.call('client_unit_state', {'pid': PID, 'target': 'target'})
    if not t.get('present') or (t.get('health') or 0) <= 0:
        m.action('cycle_hostile_forward'); time.sleep(0.8)
    l, pl = near(m, mob)
    d, e = l[0]
    left = turn_to(m, yaw_to(pl, e['position']), tol=tol)
    time.sleep(0.4)
    l, pl = near(m, mob)
    d, e = l[0]
    left = turn_to(m, yaw_to(pl, e['position']), tol=tol)
    if camera:
        m.action(camera)
        time.sleep(0.7)
    return {'mob_dist_m': round(d, 2), 'facing_error_deg': round(left, 2)}


def full_state(m, mob='Fox'):
    s = cam_state()
    l, pl = near(m, mob)
    tgt = None
    if l:
        d, e = l[0]
        tgt = {'name': e['name'], 'class': e['class'], 'position': e['position'], 'angles_rad': e['angles'], 'dist_xy_to_player': round(d, 2)}
    p, cm = s['player'], s['cam']
    s['camera_to_player_xy'] = round(math.hypot(cm['x'] - p['x'], cm['y'] - p['y']), 3)
    s['camera_height_above_player_feet'] = round(cm['z'] - p['z'], 3)
    s['camera_yaw_minus_player_yaw_deg'] = round(angdiff(s['ang']['z'], math.degrees(s['pyaw'])), 2)
    s['nearest_mob'] = tgt
    return s


def capture(m, name, skill_id, key, shift=False, pre=0.4, dur=3.2, mob='Fox', note='', prep=True):
    """One combat burst into reference/ingame/combat_<name>/: real_NN.png, frames.json, camera.json."""
    out = os.path.join(c.REF, 'ingame', 'combat_' + name)
    for f in (os.listdir(out) if os.path.isdir(out) else []):
        os.remove(os.path.join(out, f))
    prep_info = (ready(m, mob) or {'failed': True}) if prep else {}
    before = full_state(m, mob)
    ub = unit_summary(m)
    lp = log_size()
    t_wall = time.strftime('%Y-%m-%d %H:%M:%S')
    log = burst(out, lambda: m.key(key, 150, shift=shift), pre=pre, dur=dur)
    time.sleep(0.3)
    after = full_state(m, mob)
    ua = unit_summary(m)
    meta = {'name': name, 'skill_id': skill_id, 'key': ('SHIFT-' if shift else '') + key, 'captured': t_wall, 'note': note,
            'prepare': prep_info, 'camera_before': before, 'camera_after': after, 'units_before': ub, 'units_after': ua,
            'burst': log, 'server_log': log_since(lp)}
    c.save_json(os.path.join(out, 'camera.json'), {'before_key_press': before, 'after_burst': after})
    c.save_json(os.path.join(out, 'frames.json'), meta)
    print(name, 'frames', len(log['frames']), 'target', ub['target'].get('health'), '->', ua['target'].get('health'),
          'player', ub['player']['health'], '->', ua['player']['health'], 'cam yaw diff', before['camera_yaw_minus_player_yaw_deg'], '->', after['camera_yaw_minus_player_yaw_deg'])
    return meta


def contact(name, cols=4, w=480):
    from PIL import Image
    d = os.path.join(c.REF, 'ingame', 'combat_' + name)
    fs = sorted(f for f in os.listdir(d) if f.startswith('real_') and f.endswith('.png'))
    h = int(w * 9 / 16)
    rows = (len(fs) + cols - 1) // cols
    sheet = Image.new('RGB', (w * cols, h * rows))
    for i, f in enumerate(fs):
        sheet.paste(Image.open(os.path.join(d, f)).convert('RGB').resize((w, h)), ((i % cols) * w, (i // cols) * h))
    out = os.path.join(c.REF, '..', 'shots', 'sheet_%s.png' % name)
    sheet.save(out)
    return out


def approach(m, mob='Fox', stop=2.3, tries=6):
    """Turn to the nearest `mob` and walk (W) until it is within `stop` metres (xy)."""
    for _ in range(tries):
        l, pl = near(m, mob)
        d, e = l[0]
        turn_to(m, yaw_to(pl, e['position']), tol=3)
        l, pl = near(m, mob)
        d, e = l[0]
        if d <= stop + 0.3:
            return d
        m.key('w', int(min(1500, max(80, (d - stop) / 6.0 * 1000))))
        time.sleep(0.6 + (d - stop) / 6.0)
    l, pl = near(m, mob)
    return l[0][0]


def tdist(m):
    t = m.call('client_unit_state', {'pid': PID, 'target': 'target'})
    if not t.get('present') or (t.get('health') or 0) <= 0:
        return None, t
    return t.get('distance'), t


def ready(m, mob='Rat', max_dist=1.3, tol=4.0, attempts=14, camera='right_camera'):
    """Chase the (targeted) mob until it is within `max_dist` of the unit (the metric the skill range check uses), the
    character faces it, and the camera is snapped side-on. Returns a small dict, or None when it never worked."""
    for i in range(attempts):
        d, t = tdist(m)
        if d is None:
            m.action('cycle_hostile_forward'); time.sleep(0.7)
            d, t = tdist(m)
            if d is None:
                time.sleep(1.0); continue
        l, pl = near(m, mob)
        # the entity nearest to the player is normally the targeted one
        e = l[0][1]
        face = yaw_to(pl, e['position'])
        turn_to(m, face, tol=2.0, tries=3)
        d, t = tdist(m)
        if d is not None and d > max_dist:
            m.key('w', int(min(900, max(60, (d - 0.8) / 6.0 * 1000))))
            time.sleep(0.5 + min(0.9, (d - 0.8) / 6.0))
            continue
        l, pl = near(m, mob)
        e = l[0][1]
        err = angdiff(yaw_to(pl, e['position']), math.degrees(cam_state()['pyaw']))
        if abs(err) > tol:
            turn_to(m, yaw_to(pl, e['position']), tol=2.0, tries=3)
        if camera:
            m.action(camera); time.sleep(0.45)
        d, t = tdist(m)
        l, pl = near(m, mob)
        e = l[0][1]
        err = angdiff(yaw_to(pl, e['position']), math.degrees(cam_state()['pyaw']))
        if d is not None and d <= max_dist + 0.2 and abs(err) <= tol + 2:
            return {'unit_distance_m': round(d, 2), 'facing_error_deg': round(err, 1), 'attempt': i + 1}
    return None


def quality(meta, min_frames=8, max_gap=0.65):
    """True when a burst has enough frames and no stall (the machine and the shared MySQL are sometimes very slow)."""
    fr = meta['burst']['frames']
    after = [f for f in fr if f['t_done_s'] >= 0]
    if len(fr) < min_frames:
        return False
    gaps = [b['t_done_s'] - a['t_done_s'] for a, b in zip(fr, fr[1:])]
    return max(gaps) <= max_gap


def timed(out_dir, schedule, trigger, pre=0.5, start_index=1):
    """Screenshots at given seconds after the key press (a sparse timeline, e.g. a 10 s cast), strictly one at a time.
    `schedule` = list of target times in seconds after the key press (frames run back to back when behind schedule)."""
    os.makedirs(out_dir, exist_ok=True)
    frames = []
    t_start = time.time()
    t_key = t_key_done = None
    result = None
    i = start_index
    pending = list(schedule)
    while True:
        now = time.time()
        if t_key is None and now - t_start >= pre:
            t_key = time.time(); result = trigger(); t_key_done = time.time()
        if t_key is not None:
            if not pending:
                break
            wait = (t_key + pending[0]) - time.time()
            if wait > 0:
                time.sleep(wait)
            target_t = pending.pop(0)
        else:
            target_t = None
        path = os.path.join(out_dir, 'real_%02d.png' % i)
        t0 = time.time()
        r = subprocess.run([c.DRIVER, 'screenshot', '--pid', str(PID), path], capture_output=True)
        t1 = time.time()
        if r.returncode == 0:
            frames.append({'file': os.path.basename(path), 't_req': t0, 't_done': t1, 'target': target_t})
            i += 1
    log = [{'file': f['file'], 'target_s': f['target'], 't_req_s': round(f['t_req'] - t_key, 3), 't_done_s': round(f['t_done'] - t_key, 3)} for f in frames]
    return {'key_press_sent_at_s': 0.0, 'key_press_returned_s': round(t_key_done - t_key, 3), 'trigger_result': result, 'frames': log}
