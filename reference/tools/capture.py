"""reference/tools/capture.py -- drive the real ArcheAge 1.2 client through aaemu-clientdriver.

  python -I capture.py --pid PID api                  # widget_methods.json, x2_globals.json, ui_data.json
  python -I capture.py --pid PID hooks                # load lib.lua + install the call recorder
  python -I capture.py --pid PID screen <name> [--no-ui]   # real.png, ui.json, camera.json in ../<name>/
  python -I capture.py --pid PID lua "<expr>"         # run Lua in the UI state, print the result
  python -I capture.py --pid PID luas "<expr>"        # ... in the System state

Environment: AA_DRIVER_EXE (aaemu-clientdriver.exe) and AADRIVER_DLL (x86 aadriver.dll) from the AAEmu
server's tools; for `launch` also AA_SERVER_DIR (the server install with bin/aaemu-launcher.exe and logs/),
AA_PAK (the client's game_pak, its bin32/archeage.exe is started), AA_LOGIN and AA_PASSWORD_FILE.
All output is UTF-8 JSON, pretty printed, keys sorted.
"""
import argparse, json, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.dirname(HERE)
DRIVER = os.environ.get('AA_DRIVER_EXE', 'aaemu-clientdriver.exe')
SERVER = os.environ.get('AA_SERVER_DIR', '.')


def drv(pid, args, stdin=None, timeout=120):
    r = subprocess.run([DRIVER] + args, input=stdin, capture_output=True, timeout=timeout)
    out = r.stdout.decode('utf-8', 'replace')
    if r.returncode != 0:
        raise RuntimeError('driver %s failed (%d): %s %s' % (args[:3], r.returncode, out[:300], r.stderr.decode('utf-8', 'replace')[:300]))
    return out


def lua(pid, code, target='ui', timeout=120):
    return drv(pid, ['lua', '-t', target, '--pid', str(pid), '-'], stdin=code.encode('utf-8'), timeout=timeout)


def lua_file(pid, name, target='ui'):
    with open(os.path.join(HERE, name), 'rb') as f:
        return lua(pid, f.read().decode('utf-8'), target)


def lua_json(pid, code, target='ui'):
    s = lua(pid, code, target).strip('\r\n')
    return json.loads(s)


def save_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(obj, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write('\n')
    print('wrote', os.path.relpath(path, REF), os.path.getsize(path), 'bytes')


def load_libs(pid):
    for t in ('ui', 'system', 'third'):
        lua_file(pid, 'lib.lua', t)


def shrink_png(path):
    """Lossless: the back buffer comes as RGBA with a constant alpha of 255; store RGB, zlib level 9."""
    from PIL import Image
    import numpy as np
    im = Image.open(path)
    if im.mode == 'RGBA' and bool((np.asarray(im)[:, :, 3] == 255).all()):
        im.convert('RGB').save(path, optimize=True, compress_level=9)


def screenshot(pid, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    drv(pid, ['screenshot', '--pid', str(pid), path])
    shrink_png(path)
    with open(path, 'rb') as f:
        head = f.read(32)
    w = int.from_bytes(head[16:20], 'big'); h = int.from_bytes(head[20:24], 'big')
    print('wrote', os.path.relpath(path, REF), '%dx%d' % (w, h))
    return w, h


CAMERA_LUA = r'''
local function first(...) return (select(1, ...)) end
local o = { pos = first(System.GetViewCameraPos()), dir = first(System.GetViewCameraDir()),
  angles_deg = first(System.GetViewCameraAngles()), fov_rad = System.GetViewCameraFov(), cvars = {} }
if o.fov_rad then o.fov_deg = o.fov_rad * 180 / math.pi end
for _, n in ipairs{ 'r_width', 'r_height', 'r_fullscreen', 'r_driver', 'r_colorbits', 'r_vsync', 'sys_spec_full',
  'e_time_of_day', 'cl_fov', 'r_aspectratio', 'r_hdrrendering', 'e_view_dist_ratio' } do
  local ok, v = pcall(System.GetCVar, n)
  if ok then o.cvars[n] = v end
end
return __ref.json(o)
'''

PLAYER_LUA = r'''
local o = {}
local ok, name = pcall(function() return X2Unit:UnitName('player') end)
if ok and name then
  o.name = name
  local r = { pcall(function() return X2Unit:GetUnitWorldPosition('player') end) }
  if r[1] then o.world_pos = { r[2], r[3], r[4], r[5] } end
  local z = { pcall(function() return X2World:GetZoneText() end) }
  if z[1] then o.zone_text = z[2] end
  local w = { pcall(function() return X2World:GetCurrentWorldName() end) }
  if w[1] then o.world_name = w[2] end
  local f = { pcall(function() return X2:GetCurrentWorldId() end) }
  if f[1] then o.world_id = f[2] end
end
return __ref.json(o)
'''


PLAYER_ENTITY_LUA = r'''
local name = %r
for _, e in pairs(System.GetEntities() or {}) do
  if e:GetName() == name then
    local p, a = e:GetWorldPos(), e:GetAngles()
    return __ref.json({ class = e.class, position = p, angles_rad = a, terrain_height = System.GetTerrainElevation(p) })
  end
end
return ''
'''

def cmd_api(pid, suffix='', widgets=True):
    load_libs(pid)
    if widgets:
        # widget classes ------------------------------------------------------------------------
        probe = lua_json(pid, 'return __ref.json(__ref.probe_classes())')
        live = lua_json(pid, '''local R=__ref local order=R.enumerate(nil,10) local g={}
    for _,rec in ipairs(order) do local m=getmetatable(rec.obj).__index local e=g[m]
     if not e then e={count=0,paths={},kind=R.kind_of(rec.obj),class=R.class_of[m]} g[m]=e end
     e.count=e.count+1 if #e.paths<3 and rec.paths[1] then e.paths[#e.paths+1]=rec.paths[1] end end
    local out={} for m,e in pairs(g) do local n={} for k in pairs(m) do if k~="__index" then n[#n+1]=k end end table.sort(n) e.methods=n out[#out+1]=e end
    return R.json(out)''')
        live_count = {}
        for e in live:
            if e.get('class'):
                live_count[e['class']] = e['count']
        wt = {}
        for name, ms in probe['widgets'].items():
            wt[name] = {'methods': ms, 'method_count': len(ms), 'live_instances': live_count.get(name, 0)} if isinstance(ms, list) else ms
        dt = {}
        for name, ms in probe['drawables'].items():
            dt[name] = {'methods': ms, 'method_count': len(ms), 'live_instances': live_count.get('drawable:' + name, 0)} if isinstance(ms, list) else ms
        common = None
        for v in wt.values():
            if 'methods' in v:
                s = set(v['methods'])
                common = s if common is None else common & s
        unnamed = [{'kind': e['kind'], 'method_count': len(e['methods']), 'live_instances': e['count'], 'sample_paths': e['paths'], 'methods': e['methods']}
                   for e in live if not e.get('class')]
        out = {
            'note': 'Per class, the method list of metatable.__index. Every object of a class shares one flat method table '
                    '(the metatable is its own __index; there is no further __index chain and no Lua-level inheritance: each class '
                    'simply lists all methods, including the ~90 common widget methods). A widget is a Lua table {__this = userdata, '
                    '<script-defined fields>}. Drawables (image/color/text/ninepart/threepart/iconimage/effect) and the text style object '
                    '(.style / .titleStyle) are the same kind of table. GetId() is a creation counter (hex string) for widgets and '
                    'meaningless for drawables/styles. Widget NAMES, textures, coords, colours, fonts, anchors and layers have no getters.',
            'create_widget': "UIParent:CreateWidget(type, name, parentName='UIParent') -> widget or nil (unknown type); "
                             "widget:CreateChildWidget(type, name, ...)",
            'widget_types': wt,
            'widget_types_not_found': probe['widget_failed'],
            'widget_types_crash_client': ['worldmap (native exception when created through UIParent:CreateWidget on its own)'],
            'drawable_types': dt,
            'drawable_creators': {
                'image': "widget:CreateImageDrawable(texturePath, layer)", 'color': "widget:CreateColorDrawable(r,g,b,a,layer)",
                'text': "widget:CreateTextDrawable(fontPath, fontSize, layer)", 'ninepart': "widget:CreateNinePartDrawable(texturePath, layer)",
                'threepart': "widget:CreateThreePartDrawable(texturePath, layer)", 'iconimage': "widget:CreateIconImageDrawable(texturePath, layer)",
                'effect': "widget:CreateEffectDrawable(texturePath, layer)",
                'layers': "'background' (also seen: 'artwork', 'overlay'); see ui.json calls[] for what the scripts use"},
            'textstyle': {'methods': probe.get('textstyle', []), 'note': 'object behind widget.style / titleStyle; created natively with label/button/textbox'},
            'methods_common_to_all_widget_types': sorted(common or []),
            'other_live_classes_without_a_creation_name': unnamed,
        }
        save_json(os.path.join(REF, 'api', 'widget_methods.json'), out)

    # globals ------------------------------------------------------------------------------
    g = {'ui_state': lua_json(pid, open(os.path.join(HERE, 'globals.lua'), encoding='utf-8').read(), 'ui'),
         'system_state': lua_json(pid, open(os.path.join(HERE, 'globals.lua'), encoding='utf-8').read(), 'system'),
         'third_state': lua_json(pid, open(os.path.join(HERE, 'globals.lua'), encoding='utf-8').read(), 'third')}
    stage = lua(pid, 'return X2LoginCharacter:GetCurrentStage(), tostring(X2:IsEnteredWorld())').strip()
    g['captured_at_login_stage'] = stage
    save_json(os.path.join(REF, 'api', 'x2_globals%s.json' % suffix), g)

    s = lua(pid, open(os.path.join(HERE, 'data_tables.lua'), encoding='utf-8').read())
    save_json(os.path.join(REF, 'api', 'ui_data%s.json' % suffix), json.loads(s))


def cmd_screen(pid, name, do_ui=True, tag=None, camera=None):
    """Capture <name>/real[_tag].png, ui[_tag].json and (untagged, or camera=True) camera.json."""
    d = os.path.join(REF, name)
    os.makedirs(d, exist_ok=True)
    sfx = '_' + tag if tag else ''
    shot_name = 'real%s.png' % sfx
    w, h = screenshot(pid, os.path.join(d, shot_name))
    if do_ui:
        lua_file(pid, 'lib.lua')  # idempotent
        s = lua_file(pid, 'snapshot.lua')
        ui = json.loads(s)
        ui['meta']['screen_name'] = name + (' (%s)' % tag if tag else '')
        ui['meta']['screenshot'] = {'file': shot_name, 'width': w, 'height': h}
        ui['meta']['captured'] = time.strftime('%Y-%m-%d %H:%M:%S')
        save_json(os.path.join(d, 'ui%s.json' % sfx), ui)
    if camera is None:
        camera = not tag
    if camera:
        lua_file(pid, 'lib.lua', 'system')
        cam = json.loads(lua(pid, CAMERA_LUA, 'system').strip())
        ply = json.loads(lua(pid, PLAYER_LUA, 'ui').strip() or '{}')
        if ply:
            cam['player'] = ply
            if ply.get('name'):
                ent = lua(pid, PLAYER_ENTITY_LUA % ply['name'], 'system').strip()
                if ent:
                    cam['player']['entity'] = json.loads(ent)
        save_json(os.path.join(d, 'camera%s.json' % sfx), cam)


def _gray(path, size=(240, 135)):
    from PIL import Image
    import numpy as np
    return np.asarray(Image.open(path).convert('L').resize(size), dtype=float)


def wait_settled(pid, max_s=20.0, thresh=0.6, need=2, tmp=None):
    """Block until consecutive screenshots (0.8 s apart) differ by < thresh mean grey levels, need times in a row."""
    tmp = tmp or os.path.join(os.environ.get('TEMP', '.'), 'aa_settle_%d.png' % pid)
    t0 = time.time(); prev = None; ok = 0; last = None
    while time.time() - t0 < max_s:
        drv(pid, ['screenshot', '--pid', str(pid), tmp])
        cur = _gray(tmp)
        if prev is not None:
            last = float(abs(cur - prev).mean())
            ok = ok + 1 if last < thresh else 0
            if ok >= need:
                return round(time.time() - t0, 1), last
        prev = cur
        time.sleep(0.8)
    return round(time.time() - t0, 1), last


BOOT_TAIL = r'''
local late = backgroundWindow ~= nil
local n = __ref.install_hooks()
return n .. ',late=' .. tostring(late) .. ',stage=' .. tostring(X2LoginCharacter:GetCurrentStage())
'''


def cmd_boot(pid, width, height, timeout=240):
    """Poll until the UI Lua state answers, then (as the very first request) install lib.lua + the call
    recorder, and resize the client with r_width/r_height. Run right after launching the client."""
    lib = open(os.path.join(HERE, 'lib.lua'), encoding='utf-8').read()
    lib = lib[:lib.rindex('return ')]
    code = lib + BOOT_TAIL
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            r = subprocess.run([DRIVER, 'lua', '-t', 'ui', '--pid', str(pid), '-'], input=code.encode(), capture_output=True, timeout=60)
        except subprocess.TimeoutExpired:
            continue
        if r.returncode == 0:
            print('hooks installed after %.1fs: %s' % (time.time() - t0, r.stdout.decode().strip()))
            break
        time.sleep(0.05)
    else:
        raise SystemExit('UI state never answered')
    print(lua(pid, 'System.SetCVar("r_width", %d) System.SetCVar("r_height", %d) return System.GetCVar("r_width"), System.GetCVar("r_height")' % (width, height), 'system').strip())


WATCH_CHECK = "if rawget(_G, '__ref') and __ref.hooks_installed_at then return 'ok' end return 'missing'"
WATCH_TAIL = r'''
local late = {}
for _, n in ipairs{'backgroundWindow', 'characterSelectWindow', 'characterCreateWindow', 'loginStageBlackWnd'} do
  if rawget(_G, n) ~= nil then late[#late + 1] = n end
end
local n = __ref.install_hooks()
__ref.epoch = (__ref.epoch or 0) + 1
return n .. ',existing=' .. table.concat(late, '+') .. ',stage=' .. tostring(X2LoginCharacter:GetCurrentStage())
'''


def cmd_watch(pid, width, height, interval=0.02):
    """Keep the call recorder installed: the client rebuilds its UI Lua state on stage changes (the
    globals vanish), so poll fast and re-install as soon as __ref is gone. Runs until killed or the client exits."""
    lib = open(os.path.join(HERE, 'lib.lua'), encoding='utf-8').read()
    lib = lib[:lib.rindex('return ')]
    code = lib + WATCH_TAIL
    installs = 0
    sized = False
    t0 = time.time()
    while True:
        try:
            r = subprocess.run([DRIVER, 'lua', '-t', 'ui', '--pid', str(pid), WATCH_CHECK], capture_output=True, timeout=60)
        except subprocess.TimeoutExpired:
            continue
        if r.returncode != 0:
            time.sleep(0.05)
            if not subprocess.run([DRIVER, 'list'], capture_output=True).stdout.decode().count('pid %d ' % pid) and installs:
                print('client gone'); return
            continue
        if r.stdout.decode().strip() == 'ok':
            time.sleep(interval); continue
        r = subprocess.run([DRIVER, 'lua', '-t', 'ui', '--pid', str(pid), '-'], input=code.encode(), capture_output=True, timeout=60)
        installs += 1
        print('%7.2fs install #%d: %s %s' % (time.time() - t0, installs, r.stdout.decode().strip(), r.stderr.decode().strip()), flush=True)
        if not sized and width:
            lua(pid, 'System.SetCVar("r_width", %d) System.SetCVar("r_height", %d)' % (width, height), 'system'); sized = True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--pid', type=int, default=0)
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('api'); p.add_argument('--suffix', default=''); p.add_argument('--no-widgets', action='store_true')
    sub.add_parser('hooks')
    p = sub.add_parser('watch'); p.add_argument('--width', type=int, default=1920); p.add_argument('--height', type=int, default=1080)
    p = sub.add_parser('boot'); p.add_argument('--width', type=int, default=1920); p.add_argument('--height', type=int, default=1080)
    p = sub.add_parser('screen'); p.add_argument('name'); p.add_argument('--no-ui', action='store_true'); p.add_argument('--tag'); p.add_argument('--camera', action='store_true', default=None)
    p = sub.add_parser('create-sweep'); p.add_argument('--races', default='1,2,3,4'); p.add_argument('--genders', default='1,2'); p.add_argument('--steps', default='1,2,3')
    p = sub.add_parser('launch')
    p = sub.add_parser('to-create')
    p = sub.add_parser('click'); p.add_argument('x', type=int); p.add_argument('y', type=int); p.add_argument('--right', action='store_true')
    p = sub.add_parser('frames'); p.add_argument('outdir'); p.add_argument('--seconds', type=float, default=50)
    p = sub.add_parser('settle'); p.add_argument('--max', type=float, default=20)
    p = sub.add_parser('lua'); p.add_argument('code')
    p = sub.add_parser('luas'); p.add_argument('code')
    a = ap.parse_args()
    if a.cmd == 'launch':
        launch(); return
    if not a.pid and os.path.exists(PIDFILE): a.pid = int(open(PIDFILE).read())
    if a.cmd == 'api': cmd_api(a.pid, a.suffix, not a.no_widgets)
    elif a.cmd == 'hooks':
        load_libs(a.pid); print('hooks wrapped:', lua(a.pid, 'return __ref.install_hooks()').strip())
    elif a.cmd == 'watch': cmd_watch(a.pid, a.width, a.height)
    elif a.cmd == 'boot': cmd_boot(a.pid, a.width, a.height)
    elif a.cmd == 'screen': cmd_screen(a.pid, a.name, not a.no_ui, a.tag, a.camera)
    elif a.cmd == 'create-sweep': sweep_create(a.pid, [int(x) for x in a.races.split(',')], [int(x) for x in a.genders.split(',')], [int(x) for x in a.steps.split(',')])
    elif a.cmd == 'to-create': to_create(a.pid)
    elif a.cmd == 'click': print(click(a.pid, a.x, a.y, 'right_click' if a.right else 'click'))
    elif a.cmd == 'frames': grab_frames(a.pid, a.outdir, a.seconds)
    elif a.cmd == 'settle': print(wait_settled(a.pid, a.max))
    elif a.cmd == 'lua': print(lua(a.pid, a.code))
    elif a.cmd == 'luas': print(lua(a.pid, a.code, 'system'))



def mcp_call(tool, args, timeout=40):
    """One-shot call of an aaemu-clientdriver MCP tool (client_pointer, client_input, ...)."""
    proc = subprocess.Popen([DRIVER, 'mcp', '--dll', os.environ['AADRIVER_DLL']], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    def send(msg):
        proc.stdin.write((json.dumps(msg) + chr(10)).encode()); proc.stdin.flush()
    def recv():
        return json.loads(proc.stdout.readline().decode())
    try:
        send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'ref', 'version': '1'}}})
        recv()
        send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        send({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': tool, 'arguments': args}})
        r = recv()
        return r
    finally:
        proc.kill()


def click(pid, x, y, action='click'):
    """Real mouse click through the client's own input handler (client pixels)."""
    r = mcp_call('client_pointer', {'pid': pid, 'action': 'move', 'x': x, 'y': y})
    time.sleep(0.3)
    r = mcp_call('client_pointer', {'pid': pid, 'action': action, 'x': x, 'y': y})
    return r


LAUNCHER = os.path.join(SERVER, 'bin', 'aaemu-launcher.exe')
PIDFILE = os.path.join(SERVER, 'logs', 'ref-client.pid')
LAUNCH_LOG = os.path.join(SERVER, 'logs', 'launcher-ref.log')


def launch(width=1920, height=1080):
    """Start my own client against the local server (ticket login, nothing typed), wait for the Lua driver,
    force the resolution. Prints and returns the pid."""
    import re
    pw = os.environ['AA_PASSWORD_FILE']
    client = os.path.join(os.path.dirname(os.environ['AA_PAK']), 'bin32', 'archeage.exe')
    login = os.environ.get('AA_LOGIN', '127.0.0.1:1237')
    log = open(LAUNCH_LOG, 'wb')
    subprocess.Popen([LAUNCHER, '--client', client, '--login', login, '--user', os.environ.get('AA_USER', 'clienttest'),
                      '--password-file', pw, '--driver-dll', os.environ['AADRIVER_DLL'], '--multi-client'], stdout=log, stderr=log)
    t0 = time.time(); pid = None
    while time.time() - t0 < 60 and not pid:
        time.sleep(1)
        m = re.search(r'client started \(pid (\d+)\)', open(LAUNCH_LOG, errors='replace').read())
        if m: pid = int(m.group(1))
    if not pid: raise SystemExit('client did not start')
    open(PIDFILE, 'w').write(str(pid))
    while time.time() - t0 < 300:
        try:
            lua(pid, 'return 1'); break
        except Exception:
            time.sleep(0.3)
    lua(pid, 'System.SetCVar("r_width", %d) System.SetCVar("r_height", %d)' % (width, height), 'system')
    print('pid', pid, 'ready after %.0fs' % (time.time() - t0), flush=True)
    return pid


def wait_stage(pid, stages, timeout=120):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            st = int(lua(pid, 'return X2LoginCharacter:GetCurrentStage()').strip())
            if st in stages: return st
        except Exception:
            pass
        time.sleep(1)
    raise SystemExit('stage %s not reached' % (stages,))


def to_create(pid):
    """World select -> (0 characters) character create step 1."""
    wait_stage(pid, (3,), 180); time.sleep(3)
    lua(pid, 'backgroundWindow.mainWindow:SelectByDataKey(1) X2World:EnterWorld(1,0)')
    st = wait_stage(pid, (4, 5), 120)
    print('stage', st, flush=True)



# ---------------------------------------------------------------------------------------------
# Flow helpers (character create sweep)
# ---------------------------------------------------------------------------------------------
def grab_frames(pid, outdir, seconds=50.0, quiet=True):
    """Screenshots only (no Lua: Lua calls during a stage switch can crash the client), as fast as the hook allows."""
    os.makedirs(outdir, exist_ok=True)
    t0 = time.time(); n = 0
    while time.time() - t0 < seconds:
        try:
            drv(pid, ['screenshot', '--pid', str(pid), os.path.join(outdir, 'f%03d.png' % n)])
            n += 1
        except Exception:
            time.sleep(1)
    print('frames:', n, 'in', outdir)


def create_state(pid):
    r = lua(pid, 'return characterCreate.race, characterCreate.gender, characterCreate.state, X2LoginCharacter:GetCurrentStage()').strip().split('\t')
    return r


def sweep_create(pid, races=(1, 2, 3, 4), genders=(1, 2), steps=(1, 2, 3)):
    for r in races:
        for g in genders:
            lua(pid, 'step1.content.raceBtn[%d]:OnClick("LeftButton")' % r)
            time.sleep(0.5)
            lua(pid, 'step1.content.rightPanel.genderBtn[%d]:OnClick("LeftButton")' % g)
            race, gender, state, stage = create_state(pid)
            print('== race', r, race, 'gender', g, gender, state, flush=True)
            for step in steps:
                if step == 1:
                    print('settle', wait_settled(pid, 14, 1.3), flush=True)
                elif step == 2:
                    lua(pid, 'characterCreate:OnNextStep()'); time.sleep(3.0)
                    print('settle', wait_settled(pid, 12, 2.0), flush=True)
                else:
                    lua(pid, 'characterCreate:OnNextStep()'); time.sleep(9.0)
                    print('settle', wait_settled(pid, 12, 1.5), flush=True)
                name = 'character_create/step%d_%s_%s' % (step, race, gender)
                cmd_screen(pid, name)
            for _ in range(len(steps) - 1):
                lua(pid, 'characterCreate:OnPrevStep()'); time.sleep(3.5)


if __name__ == '__main__':
    main()
