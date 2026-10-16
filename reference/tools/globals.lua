-- reference/tools/globals.lua  (needs lib.lua loaded first)
-- Returns JSON describing every global of the current Lua state, split into: Lua stdlib, native
-- (C function) globals, X2* API tables, other native API tables, Lua-script-defined functions and
-- Lua data tables. Run once per state (-t ui, -t system).
local R = __ref
local stdlib = {}
for n in string.gmatch('assert collectgarbage dofile error gcinfo getfenv getmetatable ipairs load loadfile loadstring module newproxy next pairs pcall print rawequal rawget rawset require select setfenv setmetatable tonumber tostring type unpack xpcall _G _VERSION string table math io os debug coroutine package arg', '%S+') do stdlib[n] = true end

local function fkind(f)
  local i = debug.getinfo(f, 'S')
  return i and i.what == 'C' and 'C' or 'Lua', i
end

local function keys_sorted(t)
  local n = {}
  for k in pairs(t) do n[#n + 1] = tostring(k) end
  table.sort(n)
  return n
end

local out = { state = nil, lua_version = _VERSION, stdlib = {}, native_functions = {}, lua_functions = {},
              x2 = {}, native_tables = {}, data_tables = {}, other = {} }
local std_members = {}
for _, lib in ipairs { 'string', 'table', 'math', 'io', 'os', 'debug', 'coroutine', 'package' } do
  local t = rawget(_G, lib)
  if type(t) == 'table' then std_members[lib] = keys_sorted(t) end
end
out.stdlib_members = std_members

for k, v in pairs(_G) do
  if type(k) == 'string' then
    local ty = type(v)
    if stdlib[k] then out.stdlib[#out.stdlib + 1] = k
    elseif k == '__ref' then
    elseif ty == 'function' then
      local kind, info = fkind(v)
      if kind == 'C' then out.native_functions[#out.native_functions + 1] = k
      else out.lua_functions[#out.lua_functions + 1] = k end
    elseif ty == 'table' then
      -- classify by the function kinds it holds
      local c, l, other = {}, {}, {}
      local ntab = 0
      for kk, vv in pairs(v) do
        local t2 = type(vv)
        if t2 == 'function' then
          local kind = fkind(vv)
          if kind == 'C' then c[#c + 1] = tostring(kk) else l[#l + 1] = tostring(kk) end
        else other[tostring(kk)] = t2 end
        if t2 == 'table' then ntab = ntab + 1 end
      end
      table.sort(c) table.sort(l)
      local entry = { native_functions = c, lua_functions = l, other = other }
      if string.sub(k, 1, 2) == 'X2' then out.x2[k] = entry
      elseif #c > 0 then out.native_tables[k] = entry
      else out.data_tables[#out.data_tables + 1] = k end
    else
      out.other[k] = ty .. ':' .. tostring(v)
    end
  end
end
table.sort(out.stdlib) table.sort(out.native_functions) table.sort(out.lua_functions) table.sort(out.data_tables)
return R.json(out)
