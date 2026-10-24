"""CAF endpoint/loop comparisons. Pure decoding, no engine/runtime parity claim."""
import json, math
from probe import OUT, clip, read, write, sample, dist, angle, sha

def measure():
    doc=json.loads((OUT/'assets.json').read_text(encoding='utf-8'))
    cache={}; rows=[]; raw=[]
    def load(path):
        if path not in cache: cache[path]=clip(path)
        return cache[path]
    for char in doc['characters']:
        byname={c['name']:c for c in char['clips'] if not c.get('missing')}
        summaries=[]; joins=[]
        for c in byname.values():
            meta,tr=load(c['path']); scale=c['position_scale']; rootid=char['skeleton']['selected_bones'][0]['controller']
            r=tr[rootid]; poses=[]; turns=[]
            for cid,t in tr.items():
                if t['pos']: poses.append((dist(t['pos'][0],t['pos'][-1])*scale,cid))
                if t['rot']: turns.append((angle(t['rot'][0],t['rot'][-1]),cid))
            maxp=max(poses,default=(0,None)); maxq=max(turns,default=(0,None))
            summaries.append({'name':c['name'],'path':c['path'],'controllers':len(tr),'skipped':meta['skipped'],'root_track_format':r['format'],'root_position_keys':len(r['pos']),'root_rotation_keys':len(r['rot']),'max_controller_position_endpoint_gap_m':maxp[0],'worst_position_controller':maxp[1],'max_controller_rotation_endpoint_gap_degrees':maxq[0],'worst_rotation_controller':maxq[1],'locked_vs_unlocked_root_max_difference_m':max(dist(r['pos'][0],p) for p in r['pos'])*scale,'native_identity_root_rotation_max_difference_degrees':max(angle(q,[0,0,0,1]) for q in r['rot'])})
        pairs=[('fist_ac_sit_down_start','fist_ac_sit_down_loop'),('fist_ac_sit_down_loop','fist_ac_sit_up'),('fist_ac_sit_down_loop','music_co_sk_sit_down_piano_start'),('music_co_sk_sit_down_piano_start','music_co_sk_sit_down_piano_cast'),('music_co_sk_sit_down_piano_cast','fist_ac_sit_down_loop'),('fist_ac_sit_down_loop','music_co_sk_sit_down_drum_start'),('music_co_sk_sit_down_drum_start','music_co_sk_sit_down_drum_cast'),('music_co_sk_sit_down_drum_cast','fist_ac_sit_down_loop')]
        for a,b in pairs:
            if a not in byname or b not in byname:
                joins.append({'from':a,'to':b,'unresolved_CAL_name':True}); continue
            ca,cb=byname[a],byname[b]; _,ta=load(ca['path']); _,tb=load(cb['path']); gaps=[]
            for bone in char['skeleton']['selected_bones']:
                cid=bone['controller']; a1=ta.get(cid); b1=tb.get(cid)
                if not a1 or not b1: continue
                pa=sample(a1['pt'],a1['pos'],ca['duration']); pb=sample(b1['pt'],b1['pos'],0)
                qa=sample(a1['rt'],a1['rot'],ca['duration'],True); qb=sample(b1['rt'],b1['rot'],0,True)
                gaps.append({'bone':bone['name'],'position_gap_m':dist([v*ca['position_scale'] for v in pa],[v*cb['position_scale'] for v in pb]) if pa and pb else None,'rotation_gap_degrees':angle(qa,qb) if qa and qb else None})
            joins.append({'from':a,'to':b,'selected_bone_gaps':gaps})
        if char['race']=='hariharan':
            path=f"animations/hariharan/{char['gender']}/music/music_co_sk_sit_down_drum_start.caf"
            meta,tr=load(path); root=tr[0x78561a24]
            raw.append({**meta,'missing_from_resolved_base_CAL':True,'root_first':root['pos'][0],'root_last':root['pos'][-1],'root_max_displacement':max(dist(root['pos'][0],x) for x in root['pos']),'root_max_rotation_from_first_degrees':max(angle(root['rot'][0],x) for x in root['rot']),'not_a_runtime_fallback':'Existence alone does not authorize bypassing native CAL lookup.'})
        rows.append({'character':char['race']+'/'+char['gender'],'clips':summaries,'joins':joins})
    write('continuity.json',{'note':'Raw controller endpoint comparisons from original CAF bytes. Does not execute native interpolation, FIFO transitions, IK, inherited graph state, or render-location smoothing. Root/pelvis joins only; all controllers for within-clip endpoint maxima.','characters':rows,'raw_Hariharan_drum_start_not_CAL_resolved':raw,'unique_CAFs_decoded':len(cache)})
    for c in rows:
        loop=next(x for x in c['clips'] if x['name']=='fist_ac_sit_down_loop')
        print(c['character'],'loop position seam',loop['max_controller_position_endpoint_gap_m'],'rotation seam',loop['max_controller_rotation_endpoint_gap_degrees'])
    print('unique CAF',len(cache),'resolved character/clip entries',sum(len(c['clips']) for c in rows))

if __name__=='__main__': measure()
