"""Original DDS/MTL diagnostic pixels; material-slice evidence, not a native scene."""
from pathlib import Path
import io,json,math,subprocess,xml.etree.ElementTree as ET
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
OUT=Path(__file__).with_name('opacity_sampler_evidence')
SOURCES={
 'shield':'materials/effects/keffect/z_tes_shield_a_type9.mtl',
 'shield-pattern':'materials/effects/keffect/zz_shield_01_d1.mtl',
 'glass':'materials/effects/broken_glass2.mtl',
}

def pak(path):
 return subprocess.run([str(ROOT/'tools/aa-pak.exe'),'cat',path],check=True,capture_output=True).stdout

def decode(path,srgb):
 raw=pak(path)
 image=np.asarray(Image.open(io.BytesIO(raw)).convert('RGBA'),dtype=float)/255
 # Pillow quantizes BC3's interpolated alpha back to eight bits. Keep the original
 # encoded endpoints/selectors as rational alpha for the hardware-sampling model.
 if raw[84:88]==b'DXT5':
  h,w,_=image.shape;blocks_w=(w+3)//4
  for by in range((h+3)//4):
   for bx in range(blocks_w):
    start=128+16*(by*blocks_w+bx);a0,a1=raw[start:start+2]
    palette=[a0,a1]+([( (7-i)*a0+i*a1)/7 for i in range(1,7)] if a0>a1 else
     [((5-i)*a0+i*a1)/5 for i in range(1,5)]+[0,255])
    selectors=int.from_bytes(raw[start+2:start+8],'little')
    for y in range(4):
     for x in range(4):
      if bx*4+x<w and by*4+y<h:
       image[by*4+y,bx*4+x,3]=palette[(selectors>>(3*(4*y+x)))&7]/255
 if srgb:
  v=image[:,:,:3];image[:,:,:3]=np.where(v<=.04045,v/12.92,((v+.055)/1.055)**2.4)
 return image

def sample(image,uv,clamp=False,point=False):
 h,w,_=image.shape
 def pixel(x,y):
  return image[min(max(y,0),h-1),min(max(x,0),w-1)] if clamp else image[y%h,x%w]
 if point:return pixel(int(math.floor(uv[0]*w)),int(math.floor(uv[1]*h)))
 x,y=uv*[w,h]-.5;ix,iy=int(math.floor(x)),int(math.floor(y));fx,fy=x-ix,y-iy
 return ((1-fx)*(1-fy)*pixel(ix,iy)+fx*(1-fy)*pixel(ix+1,iy)
  +(1-fx)*fy*pixel(ix,iy+1)+fx*fy*pixel(ix+1,iy+1))

def srgb(v):return np.where(v<=.0031308,v*12.92,1.055*v**(1/2.4)-.055)*255

def pixels(name):return np.asarray(Image.open(OUT/(name+'.png')).convert('RGB'))

def main():
 results={}
 for case,path in SOURCES.items():
  root=ET.fromstring(pak(path));tex={t.attrib['Map']:t.attrib for t in root.find('Textures')}
  diffuse=decode(tex['Diffuse']['File'],True);opacity=decode(tex['Opacity']['File'],False)
  color=np.array([float(x) for x in root.attrib['Diffuse'].split(',')]);alpha=float(root.attrib['Opacity'])
  mod=next(t for t in root.find('Textures') if t.attrib['Map']=='Diffuse').find('TexMod')
  m=mod.attrib if mod is not None else {}
  tile=np.array([float(m.get('TileU',1)),float(m.get('TileV',1))])
  offset=np.array([float(m.get('OffsetU',0)),float(m.get('OffsetV',0))])
  # These three original materials use only native type2 motion; t=2 is fixed.
  for axis,name in enumerate(('U','V')):
   if float(m.get('TexMod_'+name+'OscillatorType',0))==2:
    offset[axis]+=tile[axis]*float(m.get('TexMod_'+name+'OscillatorRate',0))*2*float(m.get('TexMod_'+name+'OscillatorAmplitude',0))
  capture=pixels(case+'-authored');legacy=pixels(case+'-legacy');h,w,_=capture.shape
  extent=h/(3*math.tan(math.pi/8));window=1 if case=='glass' else 1.5
  errors=[];wrong=[]
  # Magnified original maps; reject points too close to primitive edges or nearest-texel ties.
  for py in range(100,h-100,17):
   for px in range(200,w-200,19):
    raw=(np.array([.5+(px+.5-w/2)/extent,.5+(py+.5-h/2)/extent])-.5)*window+.5
    if case=='glass' and min(abs(raw[0]*opacity.shape[1]-round(raw[0]*opacity.shape[1])),
       abs(raw[1]*opacity.shape[0]-round(raw[1]*opacity.shape[0])))<.02:continue
    texel=sample(diffuse,raw*tile+offset);coverage=sample(opacity,raw,clamp=case!='glass',point=case=='glass')[3]
    expected=srgb(texel[:3]*color*(alpha*texel[3]*coverage))
    errors.append(float(np.max(np.abs(expected-capture[py,px]))))
    coverage_wrong=sample(opacity,raw)[3]
    expected_wrong=srgb(texel[:3]*color*(alpha*texel[3]*coverage_wrong))
    wrong.append(float(np.max(np.abs(expected_wrong-capture[py,px]))))
  entry=dict(samples=len(errors),max_srgb_error=max(errors),mean_srgb_error=float(np.mean(errors)),
   old_repeat_linear_model_mean_error=float(np.mean(wrong)),changed_pixels=int(np.count_nonzero(np.any(capture!=legacy,axis=2))))
  assert entry['max_srgb_error']<3,(case,entry)
  assert entry['changed_pixels']>500,(case,entry)
  assert entry['old_repeat_linear_model_mean_error']>entry['mean_srgb_error']*2,(case,entry)
  if case in ('shield','glass'):
   entry['bindless_fallback_changed_pixels']=int(np.count_nonzero(np.any(capture!=pixels(case+'-ordinary'),axis=2)))
   assert entry['bindless_fallback_changed_pixels']==0,(case,entry)
  results[case]=entry
 control=int(np.count_nonzero(np.any(pixels('waterfall-control-authored')!=pixels('waterfall-control-legacy'),axis=2)))
 assert control==0;results['default_waterfall_control_changed_pixels']=control
 results['minification_changed_pixels']=int(np.count_nonzero(np.any(pixels('glass-minified-authored')!=pixels('glass-minified-legacy'),axis=2)))
 assert results['minification_changed_pixels']>500
 results['limits']='Diagnostic original-material UV squares, not reconstructed emitter geometry or a native rendered scene. Magnified pixel models include BC/filter rounding. Minification is a visual discriminator; descriptor/native controls prove MaxLOD0 rather than claiming an anisotropic pixel oracle. Default Filter=-1 retains existing filtering; native global profile and Z/depth-equal/MRT scheduling remain open.'
 (OUT/'validation.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(results,indent=2))
if __name__=='__main__':main()
