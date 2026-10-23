"""Execute original 1.2 UTF-8 length and composition-quality instructions.

Only397a8d40 (static catalog lookup) is substituted with its read-only SQLite rows.
No game process/server changes. Requires pefile/unicorn on the RE machine.
"""
from pathlib import Path
import hashlib
import sqlite3
import struct
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_ESP, UC_X86_REG_EIP

GAME = Path('D:/aa-re/client12/x2game_unpacked.dll')
COMMON = Path('D:/aa-re/client12/bin/xlcommon.dll')
assert hashlib.sha256(GAME.read_bytes()).hexdigest() == '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
u = Uc(UC_ARCH_X86, UC_MODE_32)
for path in [GAME, COMMON]:
    p = pefile.PE(str(path))
    image = p.get_memory_mapped_image()
    u.mem_map(p.OPTIONAL_HEADER.ImageBase, (len(image)+4095)&~4095)
    u.mem_write(p.OPTIONAL_HEADER.ImageBase,image)
u.mem_map(0x10000000,0x10000)
u.mem_map(0x20000000,0x10000)

def read(address): return struct.unpack('<I',u.mem_read(address,4))[0]
def write(address,*values): u.mem_write(address,struct.pack('<'+'I'*len(values),*values))

db=sqlite3.connect('file:D:/aaemu-live/rs-game/Data/compact.sqlite3?mode=ro',uri=True)
limits=[r[0] for r in db.execute('SELECT up_limit FROM expert_limits ORDER BY up_limit')]
calls=[]
def hook(_,address,size,data):
    if address==0x397a8d40:
        sp=u.reg_read(UC_X86_REG_ESP)
        grade=read(sp+4)
        calls.append(grade)
        u.reg_write(UC_X86_REG_EAX,limits[grade])
        u.reg_write(UC_X86_REG_EIP,read(sp))
        u.reg_write(UC_X86_REG_ESP,sp+4)
u.hook_add(UC_HOOK_CODE,hook)
rows=['# Original x2game851ae243 / xlcommon'+hashlib.sha256(COMMON.read_bytes()).hexdigest()]
for text in ['', 'CDE', '\u00e9\ub3c4\U0001f600', 'e\u0301', '\ub3c4'*200, '\U0001f600'*201]:
    value=text.encode('utf-8')
    u.mem_write(0x10000000,value+b'\0')
    sp=0x2000f000
    write(sp,0x1000fff0,0x10000000)
    u.reg_write(UC_X86_REG_ESP,sp)
    u.emu_start(0x3301b000,0x1000fff0,count=100000)
    rows.append('length\t'+value.hex()+'\t'+str(u.reg_read(UC_X86_REG_EAX)))
for grade in range(7):
    lower=limits[grade-1] if grade else 0
    for points in [lower,lower+1,lower+5000,limits[grade]]:
        calls.clear()
        u.reg_write(UC_X86_REG_ESP,0x2000f000)
        u.reg_write(UC_X86_REG_EBP,0x20008000)
        u.reg_write(UC_X86_REG_EBX,grade)
        u.reg_write(UC_X86_REG_ESI,points)
        u.emu_start(0x39083a8a,0x39083b10,count=1000)
        bits=read(0x20007ffc)
        assert calls==([0] if grade==0 else [0,grade-1]),calls
        rows.append(f'quality\t{grade}\t{points}\t{bits:08x}')
out=Path('crates/aa-client/src/hud/user_music/native.tsv')
out.write_text('\n'.join(rows)+'\n',encoding='utf-8')
print(f'{len(rows)-1} original-code cases -> {out}')
