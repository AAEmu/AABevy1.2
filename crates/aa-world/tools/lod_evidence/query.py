import ida_auto,ida_hexrays,ida_funcs,ida_bytes,idautils,idc
from pathlib import Path
ida_auto.enable_auto(False);ida_hexrays.init_hexrays_plugin()
p=Path('D:/AABevy1.2/crates/aa-world/tools/lod_evidence')
funcs={0x301126F0,0x30111AE0,0x30112350,0x30151BE0}
for ea in sorted(funcs):
 f=ida_funcs.get_func(ea)
 out=[str(ida_hexrays.decompile(ea))]
 for a in idautils.Heads(f.start_ea,f.end_ea):out.append('%08X %s'%(a,idc.generate_disasm_line(a,0)))
 (p/('%08X.txt'%ea)).write_text('\n'.join(out),encoding='utf-8')
idc.qexit(0)
