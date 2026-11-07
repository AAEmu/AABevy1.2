"""A stand-in for `aaemu-clientdriver mcp` for the run_original test: newline JSON-RPC on stdin / stdout.

It answers the tools run_original.py uses with canned, time-driven data: a `client_input` press makes the next
`client_trace drain` return a `press` row and a few `anim` / `fx` rows (without roles, like the original), and the next sampler
drains return a cast bar start / stop, a cooldown pair and player positions, in the exact line formats of run_original's Lua
samplers. `client_lua` is answered by looking for marker text in the code (this is not a Lua interpreter).

    python fake_driver.py mcp [--tools client_trace,client_video,...]      # tool list; leave out client_trace to test the error

Client clocks: the UI clock is wall ms since the start + 5000, the system clock is wall seconds since the start + 7.
"""
import json
import re
import sys
import time

START = time.time()
OFFSET_UI_MS = 5000.0
OFFSET_SYS_S = 7.0
ALL_TOOLS = ["client_attach", "client_status", "client_lua", "client_input", "client_chat", "client_target", "client_unit_state",
             "client_pointer", "client_trace", "client_video"]


def ui_ms(wall=None):
    return int(((wall or time.time()) - START) * 1000 + OFFSET_UI_MS)


def sys_ms(wall=None):
    return int(((wall or time.time()) - START + OFFSET_SYS_S) * 1000)


class State:
    def __init__(self):
        self.tracing = False
        self.trace_events = []        # (due wall seconds, row dict) rows for the next drains
        self.ui_events = []           # (due wall seconds, line)
        self.sys_events = []
        self.chat = []
        self.slot = 0
        self.bar = {}
        self.video = None
        self.log = []


def due(events, now):
    ready = [e for e in events if e[0] <= now]
    events[:] = [e for e in events if e[0] > now]
    return [e[1] for e in sorted(ready, key=lambda e: e[0])]


def handle_lua(st, code):
    now = time.time()
    if "GetCurrentStage" in code:
        return "0\tFakeChar"
    if "X2Time:GetUiMsec()" in code and "pcall(function()" in code and len(code) < 200:
        return str(ui_ms())
    if code.strip() == "return System.GetCurrTime()":
        return f"{now - START + OFFSET_SYS_S:.3f}"
    if "__combat_recorder = _G.__combat_recorder or" in code:
        return "hooked"
    if "o.chat={} return 1" in code:
        return "1"
    if "local o=_G.__combat_recorder if not o then return '' end" in code and "o.chat" in code:
        lines, st.chat = st.chat, []
        return "\n".join(lines)
    if "Script.SetTimer(" in code and "local function tick()" in code:
        return "installed"
    if "c[\"sp_ui\"" in code or ('"sp_ui"' in code and "table.concat" in code):
        return "\n".join(due(st.ui_events, now))
    if '"sp_sys"' in code and "table.concat" in code:
        return "\n".join(due(st.sys_events, now))
    if "GetSeamlessOrigin" in code:
        return "1000.0,2000.0,0.0"
    if "_G.SP.slot=" in code:
        st.slot = int(re.search(r"_G\.SP\.slot=(\d+)", code).group(1))
        return "1"
    if "_G.SP[" in code and "_stop" in code:
        return "1"
    if "SP.errors" in code:
        return ""
    if code.strip() == "_G.SP=nil return 1":
        return "1"
    if "SetActionSpell" in code:
        mapping = dict((int(a), int(b)) for a, b in re.findall(r"\[(\d+)\]=(\d+)", code))
        st.bar = mapping
        return "|".join(f"{slot}:true:{sid}:Skill {sid}" for slot, sid in sorted(mapping.items(), key=lambda kv: str(kv[0])))
    if "SetWatchTarget" in code:
        return "4242"
    if "CHAT_UNHOOK" in code or "o.method_hooked = nil" in code:
        return "removed"
    return ""


def schedule_cast(st, now):
    """What a press at `now` leads to, 33 ms, 40 ms ... later."""
    st.trace_events += [
        (now, {"src": "orig", "kind": "press", "wall_us": int(now * 1e6), "frame": 100, "key": "1", "slot": st.slot}),
        (now + 0.033, {"src": "orig", "kind": "anim", "wall_us": int((now + 0.033) * 1e6), "frame": 102, "inst": "0xAA01",
                       "clip": "all_idle_01", "layer": 0, "speed": 1.0, "blend": 0.2, "loop": True}),
        (now + 0.040, {"src": "orig", "kind": "anim", "wall_us": int((now + 0.040) * 1e6), "frame": 102, "inst": "0xAA01",
                       "clip": "staff_cast_spell_01", "layer": 0, "speed": 1.0, "blend": 0.1, "loop": False}),
        (now + 0.120, {"src": "orig", "kind": "fx", "wall_us": int((now + 0.120) * 1e6), "frame": 106, "name": "fx.test.bolt",
                       "phase": "spawn"}),
        (now + 0.150, {"src": "orig", "kind": "anim", "wall_us": int((now + 0.150) * 1e6), "frame": 108, "inst": "0xBB02",
                       "clip": "scarecrow_hit_01", "layer": 0, "speed": 1.0, "blend": 0.1, "loop": False}),
    ]
    st.ui_events += [
        (now + 0.045, f"C|{ui_ms(now + 0.045)}|start|Fake Cast|1500|0|"),
        (now + 0.060, f"K|{ui_ms(now + 0.060)}|{st.slot}|1000|1000|15000|15000|start"),
    ]
    st.sys_events += [
        (now + 0.100, f"P|{sys_ms(now + 0.100)}|player|10.0000|20.0000|30.0000|90.00"),
        (now + 0.300, f"P|{sys_ms(now + 0.300)}|player|13.0000|20.0000|30.0000|90.00"),
        (now + 0.350, f"P|{sys_ms(now + 0.350)}|target|10.0000|24.0000|30.0000|"),
    ]
    st.ui_events.append((now + 0.40, f"C|{ui_ms(now + 0.40)}|stop|Fake Cast|1500|350|"))


def call_tool(st, name, args):
    now = time.time()
    if name in ("client_attach", "client_window"):
        return "attached"
    if name == "client_status":
        return "pid=%s hidden=0 fake driver" % args.get("pid")
    if name == "client_lua":
        return handle_lua(st, args.get("code", ""))
    if name == "client_input":
        st.log.append(("input", args.get("key")))
        if st.tracing and args.get("key") not in ("escape",):
            schedule_cast(st, now)
        return "sent"
    if name == "client_chat":
        st.log.append(("chat", args.get("message")))
        if "/findobject" in args.get("message", ""):
            st.chat.append("1\tX2Chat.DispatchChatMessage\tBcId: 4242 npc 13013")
        return "sent"
    if name == "client_target":
        return "targeted " + str(args.get("unit_id"))
    if name == "client_unit_state":
        return json.dumps({"present": True, "name": "Forward Scarecrow" if args.get("target") == "target" else "FakeChar",
                           "health": 1000, "unit_id": "1092", "distance": 4.0})
    if name == "client_pointer":
        st.log.append(("pointer", args.get("x"), args.get("y")))
        return "clicked"
    if name == "client_trace":
        action = args.get("action")
        if action == "start":
            st.tracing = True
            return "started"
        if action == "drain":
            return "\n".join(json.dumps(r) for r in due(st.trace_events, now))
        if action == "status":
            return json.dumps({"running": st.tracing, "rows": 0, "dropped": 0})
        if action == "stop":
            st.tracing = False
            return "stopped"
    if name == "client_video":
        action, path = args.get("action"), args.get("path")
        if action == "start":
            st.video = {"path": path, "t0": time.time_ns() // 1000, "fps": args.get("fps", 30)}
            return "started"
        if action == "stop" and st.video:
            open(st.video["path"], "wb").write(b"fake mp4")
            with open(st.video["path"] + ".json", "w", encoding="utf-8") as f:
                json.dump({"t0_wall_us": st.video["t0"], "fps": st.video["fps"], "width": 1280, "height": 720, "frames": 3,
                           "dropped": 0}, f)
            return "stopped"
    raise KeyError(name)


def main(argv):
    tools = ALL_TOOLS
    if "--tools" in argv:
        tools = argv[argv.index("--tools") + 1].split(",")
    st = State()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request = json.loads(line)
        if "id" not in request:
            continue
        rid, method = request["id"], request.get("method")
        response = {"jsonrpc": "2.0", "id": rid}
        if method == "initialize":
            response["result"] = {"protocolVersion": "2025-11-25", "capabilities": {}, "serverInfo": {"name": "fake"}}
        elif method == "tools/list":
            response["result"] = {"tools": [{"name": t} for t in tools]}
        elif method == "tools/call":
            params = request["params"]
            if params["name"] not in tools:
                response["error"] = {"code": -32601, "message": "unknown tool " + params["name"]}
            else:
                try:
                    text = call_tool(st, params["name"], params.get("arguments", {}))
                    response["result"] = {"content": [{"type": "text", "text": text}], "isError": False}
                except KeyError as error:
                    response["result"] = {"content": [{"type": "text", "text": f"fake driver cannot do {error}"}], "isError": True}
        else:
            response["error"] = {"code": -32601, "message": "unknown method"}
        sys.stdout.write(json.dumps(response) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
