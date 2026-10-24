-- Native world-to-Map centres use the CURRENT Map extent (392ab830).
-- The toolkit hook uses image UV, so invert the selected original art frame.
function HUD.road_frame(level, id, formKey, size)
  local frames = HUD_ROAD_FRAMES[level] and HUD_ROAD_FRAMES[level][id]
  local f = frames and frames[size]
  local form = FORM_ROAD_MAP_LIST and FORM_ROAD_MAP_LIST[formKey .. "_" .. size]
  if f == nil and form ~= nil then f = { form[1], form[2], form[5], form[6] } end
  return f
end

function HUD.road_layout(level, id, formKey)
  if roadmap == nil then return nil, 926, 556, 100 end
  local w, h = roadmap:GetExtent()
  local size = math.floor(w * 100 / 926 + 0.5)
  if size ~= 50 and size ~= 70 and size ~= 100 then return nil, w, h, 0 end
  if math.abs(w - 926 * size / 100) > 0.02 or math.abs(h - 556 * size / 100) > 0.02 then
    return nil, w, h, 0
  end
  return HUD.road_frame(level, id, formKey, size), w, h, size
end

function HUD.road_image_point(f, w, h, u, v)
  if f == nil then return u, v end
  return (u * w - f[1]) / f[3], (v * h - f[2]) / f[4]
end
