"""GPU pixels against executed original Illum statements and original DDS texels.

Fixed render time=2, orthographic camera, no tone mapping/dither/MSAA. This tests
composition/alpha, not native full-scene lighting or exact minification filtering.
"""
import io
import json
import xml.etree.ElementTree as ET
import numpy as np
from PIL import Image
from illum_alpha_source import Native, source, extract
from capture_illum_alpha import OUT

MATERIALS = {
    'bow': ('game/objects/item/bow/bow_ean_1/bow_ean_1.mtl',0),
    'road': ('game/materials/aa_terrain/roads/road_field_dirt_201a.mtl',0),
    'hammer': ('game/objects/effects/haje_hammer_second1.mtl',0),
    'plant': ('game/objects/env/03_origin_continent/001_odeco/odeco_plants_d1.mtl',1),
    'coral': ('game/objects/env/05_nature/00_common/waterplants/common_waterbig_d02.mtl',1),
}

def load(name):
    path, sub = MATERIALS[name]
    root = ET.fromstring(source(path))
    m = root.find('SubMaterials')[sub] if root.find('SubMaterials') is not None else root
    tex = next(t for t in m.find('Textures') if t.attrib['Map']=='Diffuse')
    texpath = tex.attrib['File'].lower()
    if not texpath.startswith('game/'): texpath = 'game/'+texpath
    dest = extract(texpath)
    image = np.array(Image.open(io.BytesIO(dest.read_bytes())).convert('RGBA'))/255.
    image[:,:,:3] = np.where(image[:,:,:3] <= .04045,image[:,:,:3]/12.92,((image[:,:,:3]+.055)/1.055)**2.4)
    return m,tex.find('TexMod'),image

def sample(image, uv):
    h,w,_ = image.shape
    x,y = np.asarray(uv)*[w,h]-.5
    ix,iy = int(np.floor(x)),int(np.floor(y)); fx,fy=x-ix,y-iy
    return ((image[iy%h,ix%w]*(1-fx)+image[iy%h,(ix+1)%w]*fx)*(1-fy)
        +(image[(iy+1)%h,ix%w]*(1-fx)+image[(iy+1)%h,(ix+1)%w]*fx)*fy)

def srgb(v):
    v = np.maximum(v,0.)
    return np.minimum(255.,np.where(v<=.0031308,v*12.92,1.055*v**(1/2.4)-.055)*255.)

def pixels(name):
    return np.array(Image.open(OUT/(name+'.png')).convert('RGB'))

def changed(a,b): return int(np.count_nonzero(np.any(pixels(a)!=pixels(b),axis=2)))

def check_pixels(native, kind, capture, glow_only=False, no_colors=False, uv_window=1.):
    m,mod,image = load(kind)
    flag = int(m.attrib.get('MtlFlags','0'))
    mask = int(m.attrib['GenMask'],16 if flag&0x80000 else 10)
    params = m.find('PublicParams')
    multiplier = float(params.attrib.get('AmbientMultiplier','1')) if params is not None else 1.
    factor = np.array(list(map(float,m.attrib['Diffuse'].split(','))))
    if params is not None: factor *= float(params.attrib.get('DiffuseModifierInHDR','1'))
    opacity = float(m.attrib.get('Opacity','1'))
    cutoff = float(m.attrib.get('AlphaTest','0'))
    tiling = np.array([float(mod.attrib.get('Tile'+c,'1')) if mod is not None else 1. for c in 'UV'])
    offset = np.array([float(mod.attrib.get('Offset'+c,'0')) if mod is not None else 0. for c in 'UV'])
    actual = pixels(capture)
    errors, wrong = [],[]
    for py in range(137,585,19):
        for px in range(251,709,23):
            uv = np.array([(px+.5-240)/480,(py+.5-120)/480])
            texel = sample(image,((uv-.5)*uv_window+.5)*tiling+offset)
            vertex = np.ones(4) if no_colors else np.array([.25,.5,.75,uv[0]])
            color = np.zeros(3) if glow_only else texel[:3]*factor
            alpha, composed = native.evaluate(mask,texel,vertex,opacity,multiplier,color)
            if opacity < .999: composed *= alpha
            elif cutoff > 0 and alpha < cutoff: composed = np.zeros(3)
            expected = srgb(composed)
            errors.append(float(np.max(abs(expected-actual[py,px]))))
            # A wrong model that multiplies the glow by MatDifColor is detectable.
            if mask&0x2000:
                bad = color*(vertex[:3]**2 if mask&0x400000 else 1)+texel[:3]*texel[3]*multiplier*factor
                if cutoff > 0 and alpha < cutoff: bad = np.zeros(3)
                wrong.append(float(np.max(abs(srgb(bad)-actual[py,px]))))
    result = dict(samples=len(errors),max_srgb_error=max(errors),mean_srgb_error=float(np.mean(errors)))
    if wrong: result['incorrect_material_factored_glow_mean'] = float(np.mean(wrong))
    return result

def main():
    native = Native()
    result = {}
    for name,kind,options in [
        ('bow-quad','bow',{}),('bow-glow','bow',dict(glow_only=True)),
        ('road-window','road',dict(uv_window=.25)),('hammer-quad','hammer',{}),
        ('plant-window','plant',dict(uv_window=.25)),('coral-window','coral',dict(uv_window=.25)),
        ('bow-no-colors','bow',dict(no_colors=True))]:
        values = check_pixels(native,kind,name+'-after',**options)
        result[name] = values
        assert values['mean_srgb_error'] < .8 and values['max_srgb_error'] < 3.,(name,values)
    for a,b in [('bow-quad-after','bow-ordinary-after'),('road-quad-after','road-ordinary-after'),
                ('hammer-quad-after','hammer-prepass-after'),
                ('hammer-prepass-after','hammer-prepass-ordinary-after'),
                ('hammer-dissolve-after','hammer-dissolve-prepass-after'),
                ('coral-window-after','coral-prepass-after'),
                ('coral-prepass-after','coral-prepass-ordinary-after'),
                ('hammer-zero-after','hammer-zero-prepass-after'),
                ('bow-mesh-before','bow-mesh-zero-after'),
                ('hammer-mesh-before','hammer-mesh-zero-after'),
                ('colors-before','colors-after'),('point-before','point-after'),('waterfall-before','waterfall-after')]:
        count = changed(a,b)
        assert count == 0,(a,b,count)
        result[a+'_equals_'+b] = count
    for case in ['bow-mesh','bow-quad','road-quad','hammer-quad']:
        count = changed(case+'-before',case+'-after')
        assert count > 500,(case,count)
        result[case+'_changed_pixels'] = count
    result['limits'] = 'Original-source statement execution + base-mip bilinear DDS pixel oracle. BC decode/filter/minification error retained. No matched real-client scene, native deferred, or shadow-lighting parity claim.'
    (OUT/'validation.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))

if __name__ == '__main__': main()
