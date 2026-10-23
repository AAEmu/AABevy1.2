"""Pin the original pak shader contract and audit the small extracted terrain material library.

Reads the client exclusively through tools/aa-pak.exe. Source checks are not original
machine-code/GPU execution. The terrain_specular example separately checks GPU against DDS/CPU.
Run from the repository root after:
  tools/aa-pak.exe x materials/aa_terrain/ data/extract/aa-world/terrain-spec
"""
from pathlib import Path
import collections
import hashlib
import json
import subprocess
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "data/extract/aa-world/terrain-spec"
contracts = {
    "terrain.cfx": [
        "pPass.cGlossMap = 1;",
        "pPass.cGlossMap = GetTexture2D(glossMapSampler, baseTC.xy);",
        "fSpec = Phong(pPass.vNormal.xyz, pPass.vView.xyz, pLight.vLight.xyz, fPow);",
        "float3 cSpecular = pLight.cSpecular.xyz * fSpec*(pLight.fOcclShadow*0.8+0.2);",
        "float3 cSpecular = pPass.cSpecularAcc.xyz * pPass.cGlossMap.xyz;",
        "cSpecular.xyz *= MatSpecColor.xyz;",
        "pPass.fAlpha = sqrt( saturate(pPass.IN.Color.g) ) * AmbientOpPB.a;",
        "pPass.bDeferredSpecularShading = true;",
    ],
    "shadelib.cfi": [
        "float3 R = reflect(-L, N);",
        "return pow( saturate( dot(V, R) ) , Exp);",
        "float power = exp2( 10.h * Gloss + 1.h );",
        "float fNormFactor = power * (1.0/8.0) + (2.0/8.0);",
    ],
    "fraglib.cfi": [
        "pPass.fSpecPow = MatSpecColor.w;",
        "pPass.cBumpMap.xy += vDetailN.xy * pPass.fDetailBumpScale;",
        "pPass.cSpecularAcc = pPass.cSpecularAccRT;",
    ],
    "deferredshading.cfx": [
        "half fSpec = Blinn(vNormal, vView, vLight, vNormal.w);",
        "OUT.Specular.xyz = OUT.Diffuse.xyz * g_LightDiffuse.w * fSpec;",
    ],
}
proof = {}
for name, terms in contracts.items():
    path = "shaders/hwscripts/cryfx/" + name
    raw = subprocess.check_output([str(ROOT / "tools/aa-pak.exe"), "cat", path])
    source = raw.decode("utf-8-sig")
    compact = "".join(source.split())
    for term in terms:
        assert "".join(term.split()) in compact, (name, term)
    proof[name] = {"sha256": hashlib.sha256(raw).hexdigest(), "checks": terms}

rows = []
xml_skipped = []
files = sorted((OUT / "game/materials/aa_terrain").rglob("*.mtl"))
for path in files:
    try:
        doc = ET.fromstring(path.read_text(encoding="utf-8-sig"))
    except ET.ParseError as error:
        xml_skipped.append({"path": path.relative_to(OUT).as_posix(), "error": str(error)})
        continue
    for material in doc.iter("Material"):
        if material.get("Shader", "").lower() != "terrain.layer":
            continue
        mask = int(material.get("GenMask", "0"), 16 if int(material.get("MtlFlags", "0")) & 0x80000 else 10) & 0xffffffff
        spec = [float(v) for v in material.get("Specular", "0,0,0").split(",")]
        gloss = next((t.get("File") for t in material.iter("Texture") if t.get("Map") == "Specular"), None)
        rows.append({"path": path.relative_to(OUT).as_posix(), "mask": mask, "specular": spec, "gloss": gloss})
assert len(files) == 601, len(files)
summary = {
    "material_files": len(files), "projections": len(rows),
    "parsed_material_files": len(files) - len(xml_skipped), "xml_skipped": xml_skipped,
    "gloss_maps": sum(bool(r["gloss"]) for r in rows),
    "nonzero_specular": sum(max(r["specular"]) > 0 for r in rows),
    "nonzero_without_map": sum(not r["gloss"] and max(r["specular"]) > 0 for r in rows),
    "diffuse_alpha_unsupported": [r for r in rows if r["mask"] & 0x400],
    "gloss_alpha_power": sum(bool(r["mask"] & 0x4000) for r in rows),
    "generation_flags": dict(collections.Counter(hex(r["mask"] & 0x4780) for r in rows)),
}
(OUT / "source-contract.json").write_text(json.dumps({"source": proof, "materials": summary}, indent=2) + "\n")
print(json.dumps(summary, indent=2))
print("PASS: original forward Phong and distinct deferred Blinn source contracts")
