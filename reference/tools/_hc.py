import os, sqlite3, sys, colorsys
sys.stdout.reconfigure(encoding='utf-8')
db = sqlite3.connect('file:' + os.environ.get('AA_DB', 'compact.sqlite3') + '?mode=ro', uri=True)
c = db.cursor()
print(c.execute("select model_id,count(distinct asset_id),count(*),min(npc_only),max(npc_only) from hair_colors group by model_id").fetchall())
rows = c.execute("select id,model_id,hair_material,comment,asset_id,hair_base_color_r,hair_base_color_g,hair_base_color_b,hair_diffuse_color_r,hair_diffuse_color_g,hair_diffuse_color_b,npc_only from hair_colors where model_id=10 and hair_material like '%nu_m_hair001_hc%' order by hair_material").fetchall()
print(len(rows))
for r in rows:
    print(r[2][-8:-4], r[3], 'base', r[5:8], 'diff', r[8:11], r[11])
