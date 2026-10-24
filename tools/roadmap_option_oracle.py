"""Original option/road_map .alb execution with explicit UI endpoints and executed native checkbutton admission.
No rewritten Lua functions: host chunk sizes and f32 constants are widened only.
Getter/setter and Data execute original x2game/xlcommon code through roadmap_native_oracle.
"""
from pathlib import Path
import hashlib,json,site,struct,sys
OUT=Path('data/extract/aa-ui/minimap-audit');BASE=Path('data/extract/game/scriptsbin/x2ui/map')
sys.path.insert(0,site.getusersitepackages());sys.path.insert(0,str(OUT.resolve()));sys.path.insert(0,str(Path('data/extract/aa-ui/research-lua-deps').resolve()))
from lupa.lua51 import LuaRuntime
from roadmap_native_oracle import Native,HOST,FH,TABLE
PROOF=[]
def convert(path):
 b=path.read_bytes();pos=12;records=[]
 assert b[:12]==bytes.fromhex('1b4c75615100010404040400')
 def take(n):
  nonlocal pos
  data=b[pos:pos+n];pos+=n;assert len(data)==n;return data
 def integer():return struct.unpack('<I',take(4))[0]
 def string():
  n=integer();return struct.pack('<Q',n)+take(n)
 def function(key):
  out=string()+take(12);n=integer();code=take(n*4);out+=struct.pack('<I',n)+code
  n=integer();out+=struct.pack('<I',n)
  for _ in range(n):
   t=take(1);out+=t
   if t==b'\x01':out+=take(1)
   elif t==b'\x03':out+=struct.pack('<d',struct.unpack('<f',take(4))[0])
   elif t==b'\x04':out+=string()
   else:assert t==b'\x00'
  n=integer();out+=struct.pack('<I',n)
  for i in range(n):out+=function(key+'/'+str(i))
  n=integer();lines=take(n*4);out+=struct.pack('<I',n)+lines
  n=integer();out+=struct.pack('<I',n)
  for _ in range(n):out+=string()+take(8)
  n=integer();out+=struct.pack('<I',n)
  for _ in range(n):out+=string()
  records.append({'key':key,'instruction_count':len(code)//4,'instruction_sha256':hashlib.sha256(code).hexdigest(),'line_table_sha256':hashlib.sha256(lines).hexdigest()})
  return out
 result=bytes.fromhex('1b4c75615100010408040800')+function('main');assert pos==len(b)
 PROOF.append({'path':str(path),'sha256':hashlib.sha256(b).hexdigest(),'functions':records});return result
CHUNKS={name:convert(BASE/(name+'.alb')) for name in ['option','road_map']}
p=Path('data/extract/aa-ui/checkbutton_oracle.py');env={'__file__':str(p.resolve()),'__name__':'offline_check_definitions'}
exec(compile(p.read_text(encoding='utf8').split('\nm=Checks();typed={}')[0],str(p),'exec'),env)
CHECK=env['Checks']();m=Native()
def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def decode(obj):
 if isinstance(obj,bytes):return obj.decode()
 if hasattr(obj,'items'):
  values={decode(k):decode(v)for k,v in obj.items()}
  if values and all(type(k)is int for k in values) and set(values)==set(range(1,len(values)+1)):return [values[i]for i in range(1,len(values)+1)]
  return values
 return obj
def run(label,payload=None,injected=None,clicks=0,window_initial=True):
 lua=LuaRuntime(encoding=None,unpack_returned_tuples=True);g=lua.globals();events=[];saves=[];check_calls=[]
 load=lua.eval(b'function(bytes)local f,e=loadstring(bytes);assert(f,e);return f end')
 if payload is not None:
  m.load(payload);m.returned='unreturned';m.trace=[];m.call(0x39443a70,HOST,FH);values=m.returned
 else:
  m.load('other 17\n');values=injected
 def to_lua(v):
  if isinstance(v,dict):return lua.table_from({str(k).encode():to_lua(x)for k,x in v.items()})
  return v
 g.options=to_lua(values)
 def log(*args):events.append([decode(x)for x in args])
 def save(table):
  values=decode(table);m.tables[TABLE]=values;m.reads=[];m.call(0x39443bc0,HOST,FH,TABLE)
  saves.append({'script_fields':values,'native_data':m.serialize(),'native_reads':list(m.reads)})
 def native_check(name,value,fire,old):
  tag='boolean' if type(value)is bool else 'number' if type(value)in(int,float) else 'string' if isinstance(value,bytes) else 'nil'
  r=CHECK.typed([(tag,value),('boolean',bool(fire))],checked=bool(old))
  check_calls.append({'name':decode(name),'input':decode(value),'input_type':tag,'old':bool(old),'checked':bool(r['checked']),'event':bool(r['events'])})
  return bool(r['checked']),bool(r['events'])
 def extent(w,h):g.extent=lua.table_from([f32(w),f32(h)]);log(b'extent',f32(w),f32(h))
 def mapsize(size):g.mapSize=f32(size);log(b'map_size',f32(size))
 def reload(name):
  name=decode(name).lower()
  if name.endswith('/option.lua'):load(CHUNKS['option'])()
  elif name.endswith(('/road_map_view.lua','/road_map_list.lua','/option_view.lua')):log(b'constructor_endpoint',name)
  else:raise AssertionError(name)
 g.pylog=log;g.pysave=save;g.pycheck=native_check;g.pyextent=extent;g.pymapsize=mapsize;g.pyreload=reload
 lua.execute(b'''
 function widget(name)
  local w={name=name,handlers={},shown=true,enabled=true,checked=false}
  function w:SetHandler(ev,f)self.handlers[ev]=f end
  function w:GetParent()return parent end
  function w:RemoveAllAnchors()end
  function w:AddAnchor(...)end
  function w:IsVisible()return self.shown end
  function w:Show(v)self.shown=v;pylog('show',self.name,v)end
  function w:Enable(v)self.enabled=v;pylog('enable',self.name,v)end
  function w:SetText(v)self.text=v;pylog('text',self.name,v)end
  function w:GetChecked()return self.checked end
  function w:SetChecked(v,fire)
   local checked,event=pycheck(self.name,v,fire,self.checked)
   self.checked=checked
   if event and self.handlers.OnCheckChanged then self.handlers.OnCheckChanged()end
  end
  return w
 end
 parent=widget('parent');roadmapWindow=widget('window');roadmap=widget('roadmap')
 roadmapOptionButton=widget('optionButton');roadmapOptionButton.wnd=widget('options')
 roadmapOptionButton.wnd.chk=widget('visible');roadmapOptionButton.wnd.chk2=widget('npc')
 roadmapSizeButton=widget('size');roadmapSizeButton.enabled=false
 function roadmapWindow:SetExtent(w,h)pyextent(w,h)end
 function roadmap:SetMapSize(v)pymapsize(v)end
 function roadmap:SetRoadMapNpc(v)self.npc=v;pylog('npc_native',v)end
 function roadmap:GetTooltipController()return widget('tooltip')end
 Script={ReloadScript=function(name)pyreload(name)end}
 X2={GetRoadmapOptionUiData=function()return options end,SetRoadmapOptionUiData=function(_,o)pysave(o)end}
 locale={clock={road_map_size={'S','M','L'}}}
 function HideMapTooltip()end
 function RegistUIEvent(...)end
 function ToggleOptionFrame(...)end
 ''')
 g.roadmapWindow.shown=window_initial
 load(CHUNKS['road_map'])()
 initial={'extent':decode(g.extent),'map_size':g.mapSize,'button_text':decode(g.roadmapSizeButton.text),'window_shown':g.roadmapWindow.shown,'size_enabled':g.roadmapSizeButton.enabled,'npc_checked':g.roadmapOptionButton.wnd.chk2.checked,'npc_native_called_value':g.roadmap.npc,'setter_calls':len(saves)}
 for _ in range(clicks):g.roadmapSizeButton.OnClick(g.roadmapSizeButton,b'LeftButton')
 return {'case':label,'source_payload':payload,'window_initial_endpoint':window_initial,'getter_result':values,'injected_getter':injected,'initial':initial,'events':events,'checkbox_calls':check_calls,'setter_calls':saves,'final_native_data':m.serialize()}
if __name__=='__main__':
 rows=[]
 for size in [1,2,3]:
  for visible in [False,True]:
   for npc in [False,True]:
    payload=f'other 17\nroadmap_option\n  size {size}\n  visible {str(visible).lower()}\n  npcShow {str(npc).lower()}\n'
    row=run(f'native_s{size}_v{visible}_n{npc}',payload=payload,clicks=3 if (size==3 and visible and not npc) else 0)
    assert row['initial']['npc_checked'] is True
    rows.append(row)
 for name,payload in [('missing','other 17\n'),('empty','roadmap_option\n'),('size_only','roadmap_option size 2\n'),('visible_only','roadmap_option visible false\n'),('npc_only','roadmap_option npcShow false\n'),('unknown','roadmap_option old 8\n')]:rows.append(run(name,payload=payload))
 for value in [False,True,0,1,2,3]:rows.append(run('injected_npc_'+str(value),injected={'size':3,'visible':True,'npcShow':value}))
 for start in [False,True]:rows.append(run('visible_false_window_initial_'+str(start),injected={'size':3,'visible':False,'npcShow':False},window_initial=start))
 result={'provenance':{'chunks':PROOF,'lua_vm':decode(LuaRuntime(encoding=None).eval(b'_VERSION')),'host':'lupa lua51; original code/line bytes unchanged; chunk layout widened only','numeric_note':'float32 constants exact; UI numeric endpoints round to f32, matching native float arguments for these one-multiply controls','checkbox':'actual typed SetChecked parser/admission/setter from existing checkbutton_oracle.py definitions; original callback body executed by Lua 5.1','native_getter_setter':m.provenance,'checkbox_source_hashes':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path('D:/aa-re/client12/bin/cryscriptsystem.dll'),Path('D:/aa-re/client12/ida/xlcommon.dll'),Path('data/extract/aa-ui/checkbutton_oracle.py')]},'endpoints':['road_map_view/list/option_view constructors represented by explicit widget instances; no style/render calls','SetExtent/SetMapSize/Show/Enable/SetRoadMapNpc record calls; no GPU','Script.ReloadScript executes original option.alb when road_map.alb requests it']},'cases':rows}
 (OUT/'roadmap-option-cases.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
 print(len(rows),'original option/road_map cases')
 for row in rows:
  print(row['case'],row['getter_result'],row['initial'])
