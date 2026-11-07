"""Drive the running original 1.2 client for world-look stills: the graphics spec, the screenshot mode, the lighting state.

    py -3 tools/worldlook/orig.py --pid N spec 4              sys_spec_full (the reference look is 4, "very high")
    py -3 tools/worldlook/orig.py --pid N f12                 toggle the screenshot mode (no UI)
    py -3 tools/worldlook/orig.py --pid N state [out.json]    the lighting state and the camera as JSON
    py -3 tools/worldlook/orig.py --pid N shot out.png [state.json]
                                                             a screenshot, and the state read just before it
    py -3 tools/worldlook/orig.py --pid N lua CODE            Lua in the system state (returns the result)

The driver comes from AA_DRIVER / AA_DRIVER_DLL like the skill parity tools (it needs the `f12` key of `client_input`).
Positions in the state are game coordinates: the client's local frame shifted by the seamless origin (`--origin`, default
11264,10240; it moves in 1024 m steps when the character teleports: a GM `/move` to 15530 15325 gives 11264,11264).
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "skillparity"))

import mcp_driver  # noqa: E402

# The seamless world's local frame: the client's positions are relative to it (skill parity: the same shift).
ORIGIN = (11264.0, 10240.0)

CVARS = [
    "sys_spec_full", "e_time_of_day", "r_HDRRendering", "r_HDRLevel", "r_HDRBrightness", "r_HDRBrightOffset",
    "r_HDRBrightThreshold", "r_HDRBloomMul", "r_HDRGrainAmount", "r_HDRFlaresMul", "r_HDREyeAdaptionCache",
    "r_EyeAdaptationBase", "r_EyeAdaptationFactor", "r_EyeAdaptationLocal", "r_EyeAdaptationSpeed", "r_ColorGrading",
    "r_ColorGradingFilters", "r_ColorGradingLevels", "r_ColorGradingSelectiveColor", "r_Gamma", "r_Brightness", "r_Contrast",
    "r_PostProcessEffects", "r_Glow", "r_SunShafts", "r_ssao", "r_SSAO_amount", "r_ssaoAmount", "r_DepthOfField",
    "r_MotionBlur", "r_Rain", "e_shadows", "e_ShadowsMaxTexRes", "e_GsmRange", "e_GsmLodsNum", "e_GsmRangeStep",
    "e_fog", "e_FogVolumes", "e_sky_type", "e_sky_quality", "e_sky_update_rate", "e_view_dist_ratio",
    "e_view_dist_ratio_vegetation", "e_view_dist_ratio_detail", "e_max_view_dst", "e_vegetation_sprites_distance_ratio",
    "e_vegetation_min_size", "e_terrain_lod_ratio", "e_terrain_texture_lod_ratio", "e_detail_materials_view_dist_xy",
    "e_detail_materials_view_dist_z", "e_lod_ratio", "e_obj_quality", "e_particles_quality", "e_water_ocean",
    "e_clouds", "e_sun_clipplane_range", "r_TexMaxAnisotropy", "r_FSAA", "r_width", "r_height", "cl_fov",
    "e_time_of_day_speed", "e_dynamic_light", "r_UseEdgeAA", "r_ShadowBlur", "r_ShadowJittering",
]

STATE = r"""
local function enc(v)
  local t = type(v)
  if t == 'nil' then return 'null' end
  if t == 'number' then if v ~= v or v == math.huge or v == -math.huge then return 'null' end return string.format('%.7g', v) end
  if t == 'boolean' then return tostring(v) end
  if t == 'string' then return string.format('%q', v):gsub('\\\n', '\\n') end
  if t == 'table' then
    if v.x ~= nil and v.y ~= nil and v.z ~= nil then return '[' .. enc(v.x) .. ',' .. enc(v.y) .. ',' .. enc(v.z) .. ']' end
    local parts = {}
    for k, x in pairs(v) do parts[#parts + 1] = string.format('%q', tostring(k)) .. ':' .. enc(x) end
    return '{' .. table.concat(parts, ',') .. '}'
  end
  return string.format('%q', tostring(v))
end
local function try(f, ...) local ok, a = pcall(f, ...) if ok then return a end return nil end
local r = {}
r.sun_color = try(System.GetSunColor)
r.sky_color = try(System.GetSkyColor)
r.outdoor_ambient = try(System.GetOutdoorAmbientColor)
r.hdr_dynamic_multiplier = try(System.GetHDRDynamicMultiplier)
local hl = {}
if pcall(System.GetSkyHighlight, hl) then r.sky_highlight = hl end
r.camera_pos = try(System.GetViewCameraPos)
r.camera_dir = try(System.GetViewCameraDir)
r.camera_angles = try(System.GetViewCameraAngles)
r.camera_fov = try(System.GetViewCameraFov)
r.wind = try(System.GetWind)
r.time = try(System.GetCurrTime)
if PLAYER then
  local e = try(System.GetEntityByName, PLAYER)
  if e then r.player_pos = try(e.GetWorldPos, e) r.player_angles = try(e.GetWorldAngles, e) r.player_dir = try(e.GetDirectionVector, e, 1) end
end
r.cvars = {}
for _, n in ipairs(CVARS) do local v = try(System.GetCVar, n) if v ~= nil then r.cvars[n] = v end end
r.post = {}
for _, n in ipairs(POST) do local v = try(System.GetPostProcessFxParam, n) if v ~= nil then r.post[n] = v end end
return enc(r)
"""

# Post-effect parameters the engine may hold (unknown names read nothing).
POST = [
    "Global_Brightness", "Global_Contrast", "Global_Saturation", "Global_ColorC", "Global_ColorM", "Global_ColorY", "Global_ColorK",
    "Global_User_Brightness", "Global_User_Contrast", "Global_User_Saturation", "Global_User_ColorC", "Global_User_ColorM",
    "Global_User_ColorY", "Global_User_ColorK", "ColorGrading_Brightness", "ColorGrading_Contrast", "ColorGrading_Saturation",
    "ColorGrading_minInput", "ColorGrading_gammaInput", "ColorGrading_maxInput", "ColorGrading_minOutput",
    "ColorGrading_maxOutput", "ColorGrading_PhotoFilterColorDensity", "ColorGrading_GrainAmount", "ColorGrading_SharpenAmount",
    "SunShafts_Active", "SunShafts_Amount", "SunShafts_RaysAmount", "Dof_Active", "Dof_FocusDistance",
]


def screenshot(mcp: mcp_driver.Mcp, pid: int) -> bytes:
    """The next back buffer as PNG bytes (`client_screenshot` answers with a file path or an inline image)."""
    result = mcp.tool("client_screenshot", pid=pid)
    text = "\n".join(item["text"] for item in result.get("content", []) if item.get("type") == "text").strip()
    if text and Path(text).exists():
        return Path(text).read_bytes()
    for item in result.get("content", []):
        if item.get("type") == "image" and item.get("data"):
            return base64.b64decode(item["data"])
    raise RuntimeError("client_screenshot returned no PNG")


def lua_list(names: list[str]) -> str:
    return "{" + ",".join(json.dumps(n) for n in names) + "}"


def state(mcp: mcp_driver.Mcp, pid: int, player: str | None = None, origin: tuple[float, float] = ORIGIN) -> dict:
    who = json.dumps(player) if player else "nil"
    code = f"local CVARS = {lua_list(CVARS)} local POST = {lua_list(POST)} local PLAYER = {who}\n" + STATE
    text = mcp.text("client_lua", pid=pid, target="system", code=code)
    data = json.loads(text)
    for key in ("camera_pos", "player_pos"):
        if isinstance(data.get(key), list):
            data[key + "_local"] = data[key]
            data[key] = [data[key][0] + origin[0], data[key][1] + origin[1], data[key][2]]
    data["origin"] = list(origin)
    return data


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mcp_driver.add_driver_args(ap)
    ap.add_argument("--player", default=None, help="the character's name: its entity's position and heading go into the state")
    ap.add_argument("--origin", default="%g,%g" % ORIGIN, help="the seamless origin X,Y added to the client's local positions")
    ap.add_argument("action", choices=["spec", "f12", "state", "shot", "lua", "key"])
    ap.add_argument("rest", nargs="*")
    args = ap.parse_args(argv)
    if not args.pid:
        raise SystemExit("--pid is required (the running original client)")
    origin = tuple(float(v) for v in args.origin.split(","))
    mcp = mcp_driver.Mcp.from_args(args, None)
    try:
        if args.action == "spec":
            level = int(args.rest[0]) if args.rest else 4
            print(mcp.text("client_lua", pid=args.pid, target="system",
                           code=f"System.SetCVar('sys_spec_full', {level}) return System.GetCVar('sys_spec_full')"))
        elif args.action == "f12":
            print(mcp.text("client_input", pid=args.pid, key="f12", duration_ms=80))
        elif args.action == "key":
            # `key a 600`: hold a key (turns the character with a / d)
            print(mcp.text("client_input", pid=args.pid, key=args.rest[0], duration_ms=int(args.rest[1]) if len(args.rest) > 1 else 100))
        elif args.action == "lua":
            print(mcp.text("client_lua", pid=args.pid, target="system", code=" ".join(args.rest)))
        elif args.action == "state":
            data = state(mcp, args.pid, args.player, origin)
            text = json.dumps(data, indent=1, sort_keys=True)
            if args.rest:
                Path(args.rest[0]).write_text(text, encoding="utf-8")
            print(text)
        elif args.action == "shot":
            if not args.rest:
                raise SystemExit("shot needs an output .png")
            out = Path(args.rest[0])
            data = state(mcp, args.pid, args.player, origin)
            out.write_bytes(screenshot(mcp, args.pid))
            meta = Path(args.rest[1]) if len(args.rest) > 1 else out.with_suffix(".json")
            meta.write_text(json.dumps(data, indent=1, sort_keys=True), encoding="utf-8")
            print(out, meta)
    finally:
        mcp.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
