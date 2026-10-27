"""hair captures: helpers for reference/hair (character create step 2 and in game)."""
import os, sys, time, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture as c
PID = int(open(c.PIDFILE).read())
OUT = os.path.join(c.REF, 'hair')
os.makedirs(OUT, exist_ok=True)
def L(code, t='ui'):
    return c.lua(PID, code, t).strip()

def unit_entity_lua(extra):
    return """
for _,e in pairs(System.GetEntities() or {}) do
  local p=e:GetWorldPos()
  if tostring(e.class):find('_male') or tostring(e.class):find('_female') then
    if p.x ~= 0 or p.y ~= 0 then %s end
  end
end""" % extra

def set_yaw(yaw_rad):
    return L(unit_entity_lua("local a=e:GetAngles() e:SetAngles({x=a.x,y=a.y,z=%f})" % yaw_rad), 'system')

def get_yaw():
    return L(unit_entity_lua("local a=e:GetAngles() return string.format('%.3f',a.z)"), 'system')

def shot(name, wait=1.2):
    time.sleep(wait)
    p = os.path.join(OUT, name)
    c.screenshot(PID, p)
    return p


def to_step2(race, gender, settle=True):
    """From step 1 (or the creation entry): choose race/gender, go to step 2 (appearance), UI hidden, camera at the default zoom."""
    L("step1.content.raceBtn[%d]:OnClick('LeftButton')" % race); time.sleep(0.6)
    L("step1.content.rightPanel.genderBtn[%d]:OnClick('LeftButton')" % gender)
    time.sleep(2)
    c.wait_settled(PID, 14, 1.3)
    L('characterCreate:OnNextStep()'); time.sleep(3.0)
    c.wait_settled(PID, 12, 2.0)
    time.sleep(1.5)

def back_to_step1():
    L('characterCreate:OnPrevStep()'); time.sleep(3.5)

def ui(show):
    L("characterCreate.bgWindow.content:Show(%s) characterCreate.bottomPanel:Show(%s) return 1" % (('true',) * 2 if show else ('false',) * 2))

def hair_set(style=None, color=None):
    s = "local u=X2LoginCharacter:GetCustomizingUnit() "
    if style: s += "u:SetCustomizingHair(%d,1) " % style
    if color: s += "u:SetCustomizingHairColor(%d,1) " % color
    s += "return tostring((u:GetCustomHair()))"
    return L(s)

def zoom(n):
    """n > 0 zoom in, n < 0 zoom out (steps of ZoomIn/OutCustomizeCamera)."""
    f = 'ZoomInCustomizeCamera' if n > 0 else 'ZoomOutCustomizeCamera'
    for _ in range(abs(n)):
        L("X2LoginCharacter:GetCustomizingUnit():%s() return 1" % f); time.sleep(0.15)
    time.sleep(1.0)

def grab(path):
    c.screenshot(PID, path)
    return path

def montage(files, out, cols, cell, crop, labels=None):
    from PIL import Image, ImageDraw
    rows = (len(files) + cols - 1) // cols
    W, H = cell
    sheet = Image.new('RGB', (W * cols, H * rows))
    for i, f in enumerate(files):
        im = Image.open(f).convert('RGB').crop(crop).resize((W, H), Image.LANCZOS)
        if labels:
            ImageDraw.Draw(im).text((6, 4), str(labels[i]), fill=(255, 255, 0))
        sheet.paste(im, ((i % cols) * W, (i // cols) * H))
    sheet.save(out)


def style_montage(tag, color=22, crop=(460, 40, 1460, 1040), cols=7):
    import shutil
    n = int(L("return X2Customizer:GetNumCustomHair()"))
    tmp = os.path.join(c.REF, '..', 'shots', 'sty_' + tag); os.makedirs(tmp, exist_ok=True)
    hair_set(color=color)
    files = []
    for i in range(1, n + 1):
        hair_set(style=i); time.sleep(0.6)
        p = os.path.join(tmp, 's%02d.png' % i)
        c.drv(PID, ['screenshot', '--pid', str(PID), p])
        files.append(p)
    out = os.path.join(c.REF, '..', 'shots', 'sty_%s.png' % tag)
    montage(files, out, cols, (320, 320), crop, labels=list(range(1, n + 1)))
    return out, n


COLORS = {'black': 47, 'brown': 22, 'blond': 7, 'silver': 4, 'red': 25}

def shots(prefix, plan, z=1):
    """plan = [(style, colour_name), ...]; saves hair/<prefix>_<colour>_style<NN>.png at zoom z, UI hidden."""
    ui(False)
    zoom(z)
    log = []
    for style, col in plan:
        hair_set(style=style, color=COLORS[col]); time.sleep(1.2)
        name = '%s_%s_style%02d.png' % (prefix, col, style)
        grab(os.path.join(OUT, name))
        log.append({'file': name, 'hair_style_index': style, 'hair_color_index': COLORS[col], 'color_name': col})
    return log
