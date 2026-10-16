-- reference/tools/data_tables.lua (needs lib.lua) -- every global Lua data table (constants:
-- texture paths, font colours, sizes, locale strings, ...) as JSON. Widgets, functions and
-- userdata are skipped; depth limited; cycles cut.
local R = __ref
local skipnames = { _G = true, package = true, string = true, table = true, math = true, os = true,
  io = true, debug = true, coroutine = true, __ref = true, _LOADED = true, UIParent = true, UI = true,
  ADDON = true, Console = true, Movie = true, Script = true, Sound = true }
local function copy(v, depth, seen)
  local t = type(v)
  if t == 'number' or t == 'string' or t == 'boolean' then return v end
  if t ~= 'table' then return nil end
  if R.isw(v) or seen[v] or depth > 6 then return nil end
  seen[v] = true
  local o, n = {}, 0
  for k, x in pairs(v) do
    local kk = (type(k) == 'string' or type(k) == 'number') and k or nil
    if kk ~= nil then
      local c = copy(x, depth + 1, seen)
      if c ~= nil then o[kk] = c n = n + 1 end
    end
  end
  seen[v] = nil
  if n == 0 then return nil end
  return o
end
local out = {}
for k, v in pairs(_G) do
  if type(k) == 'string' and not skipnames[k] and string.sub(k, 1, 2) ~= 'X2' and type(v) == 'table' then
    local c = copy(v, 1, {})
    if c then out[k] = c end
  end
end
local nums = {}
for k, v in pairs(_G) do
  if type(k) == 'string' and type(v) == 'number' then nums[k] = v end
  if type(k) == 'string' and type(v) == 'string' and k ~= '_VERSION' then nums[k] = v end
end
out['__scalar_globals'] = nums
return R.json(out)
