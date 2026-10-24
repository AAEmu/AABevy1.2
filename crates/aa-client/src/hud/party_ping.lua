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
  local key=level==2 and string.upper(HUD.state.zone_key_name) or ''
  local f,w,h=HUD.road_layout(level,id,key)
  return HUD.road_image_point(f,w,h,u,v)
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
