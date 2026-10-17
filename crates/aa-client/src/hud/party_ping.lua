-- Both maps follow the authoritative SCTeamPingPos echo. No local marker is drawn on click.
AAUI.PlaceMapPing = function(level,id,u,v) return HUD.place_ping(level,id,u,v) end
AAUI.RemoveMapPing = function() return HUD.remove_ping() end
local worldMap,roadMap = AAUI.WorldMap,AAUI.RoadMap
AAUI.WorldMap = function()
  local r=worldMap()
  if r then r.ping=HUD.party_ping end
  return r
end
local function onRoadImage(level,id,u,v)
  local f=HUD_ROAD_FRAMES[level] and HUD_ROAD_FRAMES[level][id]
  local key=level==2 and string.upper(HUD.state.zone_key_name)..'_100' or ''
  local form=FORM_ROAD_MAP_LIST and FORM_ROAD_MAP_LIST[key]
  if not f and form then f={form[1],form[2],form[5],form[6]} end
  if not f then return u,v end
  return (u*926-f[1])/f[3],(v*556-f[2])/f[4]
end
AAUI.RoadMap = function()
  local r=roadMap()
  if not r then return r end
  r.ping=false
  local ping=HUD.party_ping
  if ping then
    for _,p in ipairs(ping.maps) do
      if p.level==r.level and p.id==r.zone then
        local u,v=onRoadImage(p.level,p.id,p.u,p.v)
        r.ping={level=p.level,id=p.id,u=u,v=v,version=ping.version}
        break
      end
    end
  end
  return r
end
