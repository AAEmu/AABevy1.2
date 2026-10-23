"""Execute selected original pak HLSL statements for admitted alpha/glow flags.

This is source execution, not a native GPU/deferred-scene oracle. Extract via aa-pak only.
"""
from pathlib import Path
import hashlib
import itertools
import json
import re
import subprocess
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).with_name('illum_alpha_evidence')
EXTRACT = ROOT / 'data/extract/aa-cry/illum-alpha'

def extract(path):
    dest = EXTRACT / path
    if not dest.exists():
        subprocess.run([str(ROOT/'tools/aa-pak.exe'),'x',path,str(EXTRACT)],check=True,capture_output=True)
    return dest

def source(path):
    return extract(path).read_text()

def preprocess(src, flags):
    active, stack, lines = True, [], []
    def condition(expr):
        expr = re.sub(r'%?(\w+)',lambda m: str(flags.get(m[1],False)),expr)
        return bool(eval(expr.replace('&&',' and ').replace('||',' or ').replace('!',' not ')))
    for line in src.splitlines():
        s = line.strip()
        if s.startswith(('#ifdef ', '#ifndef ', '#if ')):
            if s.startswith('#ifdef '): value = flags.get(s[7:].strip().lstrip('%'),False)
            elif s.startswith('#ifndef '): value = not flags.get(s[8:].strip().lstrip('%'),False)
            else: value = condition(s[4:])
            stack.append((active,value)); active = active and value
        elif s.startswith('#else'): active = stack[-1][0] and not stack[-1][1]
        elif s.startswith('#endif'): active = stack.pop()[0]
        elif active: lines.append(line)
    assert not stack
    return '\n'.join(lines)

class Native:
    def __init__(self):
        base = 'game/shaders/hwscripts/cryfx/'
        self.illum = source(base+'illum.cfx')
        self.frag = source(base+'fraglib.cfi')
        self.ext = source('game/shaders/illum.ext')
        self.vertex = source(base+'vertexlib.cfi')
        self.alpha = self.illum[self.illum.index('// Output alpha'):self.illum.index('#if %OPACITY_MAP',self.illum.index('// Output alpha'))]
        start = self.frag.index('    if (pPass.bVertexColors)', self.frag.index('frag_custom_end(pPass, cOut.xyz)'))
        self.composition = self.frag[start:self.frag.index('    // distance blend out',start)].replace('    X2CustomBalancePostEnd(pPass, cOut.xyz);','')
        self.cache = {}
        for name, mask in [('VERTALPHA','0x400'),('ALPHAGLOW','0x2000'),('VERTCOLORS','0x400000')]:
            assert re.search(r'Name\s*=\s*%'+name+r'\s+Mask\s*=\s*'+mask+r'\s',self.ext)
        assert '#if %VERTCOLORS || %VERTALPHA || %DIFFUSE_TEX_BLEND || %ALPHAGLOW' in self.vertex

    def program(self, mask, ambient):
        key = (mask & 0x402400, ambient)
        if key in self.cache: return self.cache[key]
        flags = dict(VERTALPHA=bool(mask&0x400),_RT_AMBIENT=ambient,_RT_ALPHABLEND=True)
        text = preprocess(self.alpha+'\n'+self.composition,flags)
        text = re.sub(r'//[^\n]*','',text)
        for a,b in {
            'pPass.cDiffuseMap.a':'diffuse[3]', 'pPass.IN.Color.w':'vertex[3]',
            'pPass.IN.Ambient.w':'opacity','pPass.bAlphaGlow':'alpha_glow',
            'pPass.bVertexColors':'vertex_rgb','pPass.IN.Color.xyz':'vertex[:3]',
            'pPass.cDiffuseMap.w':'diffuse[3]','pPass.cDiffuseMap.xyz':'diffuse[:3]',
            'pPass.fAlphaGlow_Multiplier':'multiplier','cOut.xyz':'color',
        }.items(): text = text.replace(a,b)
        text = text.replace('float fAlpha','fAlpha')
        text = re.sub(r'if \(alpha_glow\)\s*(fAlpha = vertex\[3\]);',r'if (alpha_glow) { \1; }',text)
        # The selected source contains braces plus single-line if statements.
        lines, depth, single = [], 0, False
        for token in re.split(r'([{};])',text):
            token = token.strip()
            if token == '{': depth += 1
            elif token == '}': depth -= 1
            elif token and token != ';':
                if token.startswith('if '):
                    lines.append('    '*depth+re.sub(r'^if\s*\((.*)\)$',r'if \1:',token))
                    single = token == 'if (alpha_glow)' and depth == 0
                else:
                    lines.append('    '*(depth+int(single))+token)
                    single = False
        code = '\n'.join(lines)
        self.cache[key] = compile(code,'original Illum alpha/glow statements','exec'),code
        return self.cache[key]

    def evaluate(self, mask, diffuse, vertex, opacity, multiplier, color, ambient=True):
        scope = dict(diffuse=np.array(diffuse),vertex=np.array(vertex),opacity=opacity,
            multiplier=multiplier,color=np.array(color,dtype=float),
            alpha_glow=bool(mask&0x2000),vertex_rgb=bool(mask&0x400000))
        exec(self.program(mask,ambient)[0],scope)
        return float(scope['fAlpha']),scope['color']

def main():
    native = Native()
    cases = 0
    for mask,a,v,opacity,ambient in itertools.product([0,0x400,0x2000,0x402400],[0.,.3,1.],[0.,.4,1.],[.2,1.],[False,True]):
        alpha, color = native.evaluate(mask,[.2,.4,.6,a],[.3,.5,.7,v],opacity,2.13,[.1,.2,.3],ambient)
        expected_alpha = (v if mask&0x2000 else a*(v if mask&0x400 else 1))*opacity
        expected_rgb = np.array([.1,.2,.3])* (np.array([.3,.5,.7])**2 if mask&0x400000 else 1)
        if ambient and mask&0x2000: expected_rgb += a*np.array([.2,.4,.6])*2.13
        assert abs(alpha-expected_alpha)<1e-12 and np.max(abs(color-expected_rgb))<1e-12
        cases += 1
    OUT.mkdir(exist_ok=True)
    paths = ['game/shaders/hwscripts/cryfx/'+n for n in ['illum.cfx','fraglib.cfi','vertexlib.cfi']]+['game/shaders/illum.ext']
    result = dict(scope='Selected original forward ambient alpha/composition statements; no native GPU/deferred scene execution',
        cases=cases,sha256={p:hashlib.sha256((EXTRACT/p).read_bytes()).hexdigest() for p in paths},
        alpha_source=native.alpha,composition_source=native.composition,
        translated_both=native.program(0x402400,True)[1])
    (OUT/'source_oracle.json').write_text(json.dumps(result,indent=2)+'\n')
    print(f'{cases} original-source alpha/precedence/ambient/vertex-RGB cases passed')

if __name__ == '__main__': main()
