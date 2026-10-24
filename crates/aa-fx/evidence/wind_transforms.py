"""Pinned original producer transforms for the admitted Size0 burst recipe."""
import sys,json,math,struct
from pathlib import Path
sys.dont_write_bytecode=True
from wind_area import Area
out=Path(__file__).resolve().parent
n=Area();rows=[];tsv=['# label then f32 bits: translationXYZ quaternionXYZW world_scale effect_scale offsetXYZ age native_centerXYZ native_radius native_strength']
bits=lambda x:f'{struct.unpack("<I",struct.pack("<f",x))[0]:08x}'
for label,c,q,ws,es,offset in [
 ('unit',(2,3,4),(0,0,0,1),1,1,(0,0,0)),
 ('offset-yaw-scale',(2,3,4),(0,0,math.sqrt(.5),math.sqrt(.5)),2,3,(1,2,3)),
 ('offset-tilt',(5,-7,11),(math.sin(.3),0,0,math.cos(.3)),1,1,(1,2,3)),
 ('small-positive-scale',(-10,20,-30),(0,0,0,1),.25,.5,(2,0,-1))]:
 n.u32(0x302A9730,0)
 s,p,src,fx=n.fixture(speed=78.4,size=0,now=.1,translation=c,rot=q,scale=ws,source_scale=es,offset=offset)
 n.vm.mem_write(p+612,b'\1')
 result=n.update(s);add=next(e for e in result['events'] if e['op']=='AddArea');flow=next(e for e in result['events'] if e.get('type')==12)['f32'][8]
 rows.append(dict(name=label,input=dict(translation=c,rotation=q,world_scale=ws,effect_scale=es,offset=offset,age=.1),**result))
 vals=[*c,*q,ws,es,*offset,.1,*add['pos'],add['scale'],flow]
 tsv.append(label+'\t'+'\t'.join(map(bits,vals)))
(out/'wind_transforms_native.json').write_text(json.dumps(dict(cases=rows,boundary='Original producer and CRT; geometry/physics creation interfaces recorded fixtures. Full sphere recipe Size0, speed78.4, focus gravity with fixture gravity. No solver or arbitrary cache gather.'),indent=2)+'\n',encoding='utf-8')
(out/'wind_transforms.tsv').write_text('\n'.join(tsv)+'\n',encoding='utf-8')
print('executed four native burst transform goldens')
