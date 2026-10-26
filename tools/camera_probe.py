"""Read-only probe of the running 1.2 client's follow camera (r208022): one CSV line per sample, 30 a second by default.

It only reads the client's memory (ReadProcessMemory): no debugger, no injection, nothing written to the client. Run it at the
same level as the client (a client started as administrator needs an administrator prompt). It refuses to sample when an
anti-cheat module is loaded.

    py -3 tools/camera_probe.py                         wait for archeage.exe, sample until it exits or Ctrl+C
    py -3 tools/camera_probe.py --out <file.csv> --hz 30 --seconds 600 --wait 600
    py -3 tools/camera_probe.py --check                 one sample, every field printed, then exit

Writes data/extract/aa-client/camera/native-<time>.csv unless --out says otherwise. aa-client writes the same columns with
AA_CAMERA_LOG (crates/aa-client/src/ingame/camera_log.rs), and tools/camera_compare.py sets the two side by side.

What it reads, x2game r208022:

    camera::g_camera     x2game+0xb0e2b8, a pointer to the camera object; of it
                           +0x4  mode (1 the follow camera; 4 before world entry; 6 the screenshot camera)
                           +0x20 the wheel's wanted distance, +0x24 the distance now, +0x38 the arm's length after the walls
                           +0x3c T, the point followed (the `camera` bone), +0x48 the .cdf lift, +0x58 T after the lag
                           +0x64 the camera's place, +0x70 the turn (a quaternion x y z w), +0x80 / +0x84 yaw (heading + offset)
                           +0x88 the arm's pitch, +0x8c the pitch past the arm's 50 degrees (turned into the view), +0x98 cl_fov
    net::g_connection    x2game+0xb12294; +0x10c the own unit, unit +0x3be8 its model, [[model +0x1f0] +0xc] the entity,
                           entity +0x90 the feet (else the matrix at +0x50)

Columns, game axes (x east, y north, z up), the client's local frame:

    time               seconds since the probe started
    mode               camera +0x4
    feet_x..z          the own unit's feet
    cam_x..z           camera +0x64
    target_x..z        camera +0x3c (T)
    fwd_x..z           the view's forward: +0x70 applied to +y, tilted up by +0x8c
    yaw_deg            +0x80 + +0x84
    pitch_deg          the view's pitch (asin of fwd_z; positive looks up)
    distance, wanted   +0x24, +0x20
    zoffset            +0x48
    fov_deg            +0x98 as degrees (before the aspect factor 1.05 .. 0.95 the client applies)
    cam_above_feet     cam_z - feet_z
    target_above_feet  target_z - feet_z
    look_above_feet    where the view's centre ray passes over the feet
    arm_pitch_deg      +0x88, view_extra_deg +0x8c, arm_len +0x38, lag_x..z +0x58
    feet_src           which entity field held the feet (pos90 or tm), 'none'; '?' after it: the entity's vtable was not CEntity's
    head_above_feet..toe_screen  aa-client's drawn body; empty here (the client's own frames show it)
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import math
import os
import struct
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# x2game r208022
G_CAMERA = 0xb0e2b8
G_CONNECTION = 0xb12294
CONN_UNIT = 0x10c
UNIT_MODEL = 0x3be8
MODEL_ENTITY_HOLDER = 0x1f0
HOLDER_ENTITY = 0xc
ENTITY_POS = 0x90
ENTITY_TM = 0x50
CENTITY_VTABLE = 0xc41d0     # cryentitysystem: CEntity's vtable

COLUMNS = ('time,mode,feet_x,feet_y,feet_z,cam_x,cam_y,cam_z,target_x,target_y,target_z,fwd_x,fwd_y,fwd_z,yaw_deg,pitch_deg,'
           'distance,wanted,zoffset,fov_deg,cam_above_feet,target_above_feet,look_above_feet,'
           'arm_pitch_deg,view_extra_deg,arm_len,lag_x,lag_y,lag_z,feet_src,'
           'head_above_feet,pelvis_above_feet,toe_above_feet,head_screen,pelvis_screen,toe_screen')

k32 = ctypes.WinDLL('kernel32', use_last_error=True)


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [('dwSize', wt.DWORD), ('cntUsage', wt.DWORD), ('th32ProcessID', wt.DWORD),
                ('th32DefaultHeapID', ctypes.c_size_t), ('th32ModuleID', wt.DWORD), ('cntThreads', wt.DWORD),
                ('th32ParentProcessID', wt.DWORD), ('pcPriClassBase', wt.LONG), ('dwFlags', wt.DWORD),
                ('szExeFile', wt.WCHAR * 260)]


class MODULEENTRY32W(ctypes.Structure):
    _fields_ = [('dwSize', wt.DWORD), ('th32ModuleID', wt.DWORD), ('th32ProcessID', wt.DWORD),
                ('GlblcntUsage', wt.DWORD), ('ProccntUsage', wt.DWORD), ('modBaseAddr', ctypes.c_void_p),
                ('modBaseSize', wt.DWORD), ('hModule', wt.HMODULE), ('szModule', wt.WCHAR * 256),
                ('szExePath', wt.WCHAR * 260)]


k32.CreateToolhelp32Snapshot.restype = wt.HANDLE
k32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
k32.Process32FirstW.argtypes = k32.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
k32.Module32FirstW.argtypes = k32.Module32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(MODULEENTRY32W)]
k32.OpenProcess.restype = wt.HANDLE
k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
k32.CloseHandle.argtypes = [wt.HANDLE]
k32.ReadProcessMemory.argtypes = [wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
                                  ctypes.POINTER(ctypes.c_size_t)]
k32.WaitForSingleObject.argtypes = [wt.HANDLE, wt.DWORD]
INVALID = wt.HANDLE(-1).value


def find_process(name):
    snap = k32.CreateToolhelp32Snapshot(0x2, 0)
    if snap == INVALID:
        return None
    entry = PROCESSENTRY32W()
    entry.dwSize = ctypes.sizeof(entry)
    found = None
    ok = k32.Process32FirstW(snap, ctypes.byref(entry))
    while ok:
        if entry.szExeFile.lower() == name.lower():
            found = entry.th32ProcessID
            break
        ok = k32.Process32NextW(snap, ctypes.byref(entry))
    k32.CloseHandle(snap)
    return found


def modules_of(pid):
    """Every module of the process: (name, base). Empty while the process is still starting."""
    for _ in range(20):
        snap = k32.CreateToolhelp32Snapshot(0x8 | 0x10, pid)
        if snap != INVALID:
            break
        if ctypes.get_last_error() != 24:                           # 24: the list changed while it was read
            return []
        time.sleep(0.05)
    else:
        return []
    entry = MODULEENTRY32W()
    entry.dwSize = ctypes.sizeof(entry)
    out = []
    ok = k32.Module32FirstW(snap, ctypes.byref(entry))
    while ok:
        out.append((entry.szModule, entry.modBaseAddr or 0))
        ok = k32.Module32NextW(snap, ctypes.byref(entry))
    k32.CloseHandle(snap)
    return out


def open_process(pid):
    handle = k32.OpenProcess(0x400 | 0x10 | 0x100000, False, pid)   # query, read memory, wait for its end
    if not handle:
        sys.exit(f'cannot open process {pid} (error {ctypes.get_last_error()}): if the client was started '
                 f'as administrator, run this from an administrator prompt')
    return handle


def read(handle, address, size):
    if not address or address < 0x10000:
        return None
    buffer = (ctypes.c_char * size)()
    got = ctypes.c_size_t()
    if k32.ReadProcessMemory(handle, ctypes.c_void_p(address), buffer, size, ctypes.byref(got)) and got.value == size:
        return buffer.raw
    return None


def pointer(handle, address):
    raw = read(handle, address, 4)
    return struct.unpack('<I', raw)[0] if raw else 0


def rotate(q, v):
    """The quaternion q = (x, y, z, w) applied to the vector v."""
    x, y, z, w = q
    tx = 2 * (y * v[2] - z * v[1])
    ty = 2 * (z * v[0] - x * v[2])
    tz = 2 * (x * v[1] - y * v[0])
    return (v[0] + w * tx + (y * tz - z * ty), v[1] + w * ty + (z * tx - x * tz), v[2] + w * tz + (x * ty - y * tx))


def view_forward(q, extra):
    """+y turned by the camera's quaternion, then tilted up by `extra` radians about the side (camera +0x8c)."""
    f = rotate(q, (0.0, 1.0, 0.0))
    level = math.hypot(f[0], f[1])
    if extra == 0.0 or level < 1e-6:
        return f
    pitch = math.atan2(f[2], level) + extra
    return (f[0] / level * math.cos(pitch), f[1] / level * math.cos(pitch), math.sin(pitch))


def look_above_feet(cam, fwd, feet):
    """The height over the feet at which the view's centre ray passes the feet's level distance."""
    level = math.hypot(fwd[0], fwd[1])
    if level < 1e-6:
        return None
    reach = math.hypot(feet[0] - cam[0], feet[1] - cam[1])
    return cam[2] + fwd[2] / level * reach - feet[2]


class Client:
    def __init__(self, pid, x2game, entitysys):
        self.pid, self.x2game, self.entitysys = pid, x2game, entitysys
        self.handle = open_process(pid)

    def alive(self):
        return k32.WaitForSingleObject(self.handle, 0) != 0

    def feet(self, target):
        """The own unit's feet: (x, y, z, source) or None; of the two places the entity holds, the first under the followed point."""
        g = pointer(self.handle, self.x2game + G_CONNECTION)
        unit = pointer(self.handle, g + CONN_UNIT) if g else 0
        model = pointer(self.handle, unit + UNIT_MODEL) if unit else 0
        holder = pointer(self.handle, model + MODEL_ENTITY_HOLDER) if model else 0
        entity = pointer(self.handle, holder + HOLDER_ENTITY) if holder else 0
        if not entity:
            return None
        mark = '' if not self.entitysys or pointer(self.handle, entity) == self.entitysys + CENTITY_VTABLE else '?'
        found = []
        raw = read(self.handle, entity + ENTITY_POS, 12)
        if raw:
            found.append(struct.unpack('<3f', raw) + ('pos90' + mark,))
        raw = read(self.handle, entity + ENTITY_TM, 48)
        if raw:
            m = struct.unpack('<12f', raw)
            found.append((m[3], m[7], m[11], 'tm' + mark))
        for f in found:
            # under the followed point: the same place on the level within a metre, 0.3 to 2.5 m below it
            if target is None or (math.hypot(f[0] - target[0], f[1] - target[1]) < 1.0 and 0.3 < target[2] - f[2] < 2.5):
                return f
        return None

    def sample(self):
        cam = pointer(self.handle, self.x2game + G_CAMERA)
        blob = read(self.handle, cam, 0xa0) if cam else None
        if not blob:
            return None
        f = lambda o: struct.unpack_from('<f', blob, o)[0]
        v3 = lambda o: struct.unpack_from('<3f', blob, o)
        mode = struct.unpack_from('<i', blob, 0x4)[0]
        target = v3(0x3c)
        q = struct.unpack_from('<4f', blob, 0x70)
        extra = f(0x8c)
        fwd = view_forward(q, extra)
        return {
            'mode': mode, 'cam': v3(0x64), 'target': target, 'lag': v3(0x58), 'fwd': fwd,
            'yaw': math.degrees(f(0x80) + f(0x84)), 'pitch': math.degrees(math.asin(max(-1.0, min(1.0, fwd[2])))),
            'distance': f(0x24), 'wanted': f(0x20), 'zoffset': f(0x48), 'fov': math.degrees(f(0x98)),
            'arm_pitch': math.degrees(f(0x88)), 'extra': math.degrees(extra), 'arm_len': f(0x38),
            'feet': self.feet(target if mode == 1 else None),
        }


def attach(process, wait):
    end = time.time() + wait
    while True:
        pid = find_process(process)
        if pid:
            mods = {name.lower(): base for name, base in modules_of(pid)}
            if 'x2game.dll' in mods:
                return Client(pid, mods['x2game.dll'], mods.get('cryentitysystem.dll', 0))
        if time.time() >= end:
            return None
        time.sleep(1)


def anticheat_modules(pid):
    names = {name.lower() for name, _ in modules_of(pid)}
    return sorted(n for n in names if n in ('ehsvc.dll', 'tersafe.dll', 'xigncode.dll', 'x3.xem', 'hshield.dll'))


def fmt(x):
    return '' if x is None else f'{x:.4f}'


def line(t, s):
    feet = s['feet']
    fx, fy, fz, src = feet if feet else (None, None, None, 'none')
    cam, target, fwd, lag = s['cam'], s['target'], s['fwd'], s['lag']
    look = look_above_feet(cam, fwd, (fx, fy, fz)) if feet else None
    values = [f'{t:.3f}', str(s['mode']), fmt(fx), fmt(fy), fmt(fz), *map(fmt, cam), *map(fmt, target), *map(fmt, fwd),
              fmt(s['yaw']), fmt(s['pitch']), fmt(s['distance']), fmt(s['wanted']), fmt(s['zoffset']), fmt(s['fov']),
              fmt(cam[2] - fz if feet else None), fmt(target[2] - fz if feet else None), fmt(look),
              fmt(s['arm_pitch']), fmt(s['extra']), fmt(s['arm_len']), *map(fmt, lag), src, '', '', '', '', '', '']
    return ','.join(values)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--out')
    ap.add_argument('--hz', type=float, default=30.0)
    ap.add_argument('--seconds', type=float, default=3600.0)
    ap.add_argument('--wait', type=float, default=600.0, help='seconds to wait for the client')
    ap.add_argument('--process', default='archeage.exe')
    ap.add_argument('--check', action='store_true', help='one sample, printed, then exit')
    args = ap.parse_args()

    client = attach(args.process, args.wait)
    if not client:
        print(f'no {args.process} with x2game.dll within {args.wait:.0f} s'); return 1
    loaded = anticheat_modules(client.pid)
    print(f'attached: pid {client.pid}, x2game 0x{client.x2game:x}, cryentitysystem 0x{client.entitysys:x}; '
          f'anti-cheat modules loaded: {", ".join(loaded) if loaded else "none"}', flush=True)
    if loaded:
        print('an anti-cheat module is loaded: not sampling'); return 2

    if args.check:
        s = client.sample()
        if not s:
            print('no camera object yet'); return 1
        for k, v in s.items():
            print(f'  {k:10} {v}')
        print(COLUMNS); print(line(0.0, s))
        return 0

    out = args.out or os.path.join(ROOT, 'data', 'extract', 'aa-client', 'camera', time.strftime('native-%Y%m%d-%H%M%S.csv'))
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    period = 1.0 / max(1.0, args.hz)
    start = time.perf_counter()
    count = 0
    feet_seen = None
    with open(out, 'w', encoding='utf-8', newline='\n') as f:
        f.write(f'# 1.2 native, {time.strftime("%Y-%m-%d %H:%M:%S")}, pid {client.pid}, x2game 0x{client.x2game:x}, {args.hz:g} Hz\n')
        f.write(COLUMNS + '\n')
        print(f'writing {out} (Ctrl+C to stop)', flush=True)
        try:
            next_at = start
            while client.alive() and time.perf_counter() - start < args.seconds:
                s = client.sample()
                if s:
                    f.write(line(time.perf_counter() - start, s) + '\n')
                    count += 1
                    if s['feet'] and feet_seen is None:
                        feet_seen = s['feet'][3]
                        print(f'feet found ({feet_seen}): {s["feet"][2]:.3f}, camera {s["cam"][2] - s["feet"][2]:.3f} m above them', flush=True)
                    if count % 300 == 0:
                        f.flush()
                next_at += period
                time.sleep(max(0.0, next_at - time.perf_counter()))
        except KeyboardInterrupt:
            pass
    print(f'{count} samples in {time.perf_counter() - start:.0f} s: {out}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
