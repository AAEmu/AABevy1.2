-- Appellations (titles): the natives of the title window and of the character window's title label. The Rust
-- side (appellation.rs) hands out raw rows { id, name, buff_id, buff_name, path, description, buff_kind }; these
-- build the tables the original scripts read: APPELLATION_INFO.TYPE / NAME / BUFF_INFO (1 / 2 / 3), where
-- BUFF_INFO is the buff tooltip table of the title's buff (it has the icon `path` the window draws).
local function buffInfo(raw)
  if raw.buff_id == nil then return nil end
  local tipType = raw.buff_kind == 1 and "buff" or "debuff"
  return {
    buff_id = raw.buff_id, name = raw.buff_name, path = raw.path, description = raw.description,
    tipType = tipType, category = X2Locale:LocalizeUiText(33, tipType),
  }
end
local function info(raw)
  local t = {}
  t[APPELLATION_INFO.TYPE] = raw.id
  t[APPELLATION_INFO.NAME] = raw.name
  t[APPELLATION_INFO.BUFF_INFO] = buffInfo(raw)
  return t
end
X2Player.GetAppellationCount = function(self) return HUD.appellation_count() end
-- A page of the owned titles (1 based).
X2Player.GetAppellations = function(self, page)
  local out = {}
  for i, raw in ipairs(HUD.appellation_page(page)) do out[i] = info(raw) end
  return out
end
X2Player.GetShowingAppellation = function(self)
  local raw = HUD.appellation_active()
  if raw == nil then return nil end
  return info(raw)
end
-- 0 takes the title off. True when the request went out; the answer is APPELLATION_CHANGED.
X2Player.ChangeAppellation = function(self, appellationType)
  return HUD.appellation_change(appellationType or 0)
end
