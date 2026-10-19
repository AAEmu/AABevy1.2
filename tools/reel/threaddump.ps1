# Stack dump of every thread of ONE process (a hung client of ours), no debugger needed: suspends each thread, walks its stack with
# dbghelp and prints symbols from the PDB next to the exe. Also prints each thread's state, wait reason and CPU time.
#   powershell -NoProfile -File tools/reel/threaddump.ps1 -ProcId 1234 [-Out dump.txt]
param([Parameter(Mandatory = $true)][int]$ProcId, [string]$Out = "")

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class Dump {
  [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint a, bool b, int pid);
  [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint a, bool b, uint tid);
  [DllImport("kernel32.dll")] static extern uint SuspendThread(IntPtr h);
  [DllImport("kernel32.dll")] static extern uint ResumeThread(IntPtr h);
  [DllImport("kernel32.dll")] static extern bool GetThreadContext(IntPtr h, IntPtr ctx);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("dbghelp.dll", CharSet = CharSet.Ansi)] static extern bool SymInitialize(IntPtr p, string path, bool invade);
  [DllImport("dbghelp.dll")] static extern bool SymSetOptions(uint o);
  [DllImport("dbghelp.dll")] static extern IntPtr SymFunctionTableAccess64(IntPtr p, ulong a);
  [DllImport("dbghelp.dll")] static extern ulong SymGetModuleBase64(IntPtr p, ulong a);
  [DllImport("dbghelp.dll")] static extern bool SymFromAddr(IntPtr p, ulong a, out ulong disp, IntPtr sym);
  [DllImport("dbghelp.dll")] static extern bool SymGetLineFromAddr64(IntPtr p, ulong a, out uint disp, ref LINE line);
  [DllImport("dbghelp.dll")] static extern bool StackWalk64(uint machine, IntPtr p, IntPtr t, ref FRAME f, IntPtr ctx,
      IntPtr read, SymFn fn, SymMod mb, IntPtr tr);
  delegate IntPtr SymFn(IntPtr p, ulong a);
  delegate ulong SymMod(IntPtr p, ulong a);
  [StructLayout(LayoutKind.Sequential)] struct ADDR { public ulong Offset; public uint Seg; public uint Mode; }
  [StructLayout(LayoutKind.Sequential)] struct FRAME {
    public ADDR PC, Ret, Frame, Stack, BStore; public IntPtr FuncTable;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst = 4)] public ulong[] Params;
    public int Far, Virtual;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst = 3)] public ulong[] Reserved;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst = 14)] public ulong[] KdHelp; // KDHELP64, 112 bytes
  }
  [StructLayout(LayoutKind.Sequential)] public struct LINE { public uint Size; public IntPtr Key; public uint Number; public IntPtr File; public ulong Addr; }

  static string Sym(IntPtr p, ulong a) {
    IntPtr buf = Marshal.AllocHGlobal(88 + 512);
    try {
      for (int i = 0; i < 88 + 512; i++) Marshal.WriteByte(buf, i, 0);
      Marshal.WriteInt32(buf, 0, 88);          // SizeOfStruct
      Marshal.WriteInt32(buf, 80, 512);        // MaxNameLen; the name starts at 84
      ulong d;
      string s = a.ToString("x");
      if (SymFromAddr(p, a, out d, buf)) {
        s = Marshal.PtrToStringAnsi(new IntPtr(buf.ToInt64() + 84)) + "+0x" + d.ToString("x");
      }
      LINE l = new LINE(); l.Size = (uint)Marshal.SizeOf(typeof(LINE)); uint ld;
      if (SymGetLineFromAddr64(p, a, out ld, ref l)) s += "  " + Marshal.PtrToStringAnsi(l.File) + ":" + l.Number;
      return s;
    } finally { Marshal.FreeHGlobal(buf); }
  }

  public static string Stack(int pid, uint tid, string sympath) {
    IntPtr hp = Proc;
    IntPtr ht = OpenThread(0x1F03FF, false, tid);
    StringBuilder sb = new StringBuilder();
    if (ht == IntPtr.Zero) return "  (cannot open thread)\n";
    SuspendThread(ht);
    IntPtr ctx = Marshal.AllocHGlobal(0x4D0 + 16);
    long al = (ctx.ToInt64() + 15) & ~15L;
    IntPtr c = new IntPtr(al);
    for (int i = 0; i < 0x4D0; i++) Marshal.WriteByte(c, i, 0);
    Marshal.WriteInt32(c, 0x30, 0x10001F); // CONTEXT_ALL (amd64)
    if (GetThreadContext(ht, c)) {
      FRAME f = new FRAME(); f.Params = new ulong[4]; f.Reserved = new ulong[3]; f.KdHelp = new ulong[14];
      f.PC.Offset = (ulong)Marshal.ReadInt64(c, 0xF8); f.PC.Mode = 3;
      f.Stack.Offset = (ulong)Marshal.ReadInt64(c, 0x98); f.Stack.Mode = 3;
      f.Frame.Offset = (ulong)Marshal.ReadInt64(c, 0xA0); f.Frame.Mode = 3;
      IntPtr fbuf = Marshal.AllocHGlobal(512);
      for (int n = 0; n < 40; n++) {
        if (!StackWalk64(0x8664, hp, ht, ref f, c, IntPtr.Zero, SymFunctionTableAccess64Wrap, SymGetModuleBase64Wrap, IntPtr.Zero)) break;
        if (f.PC.Offset == 0) break;
        sb.Append("    " + Sym(hp, f.PC.Offset) + "\n");
      }
      Marshal.FreeHGlobal(fbuf);
    } else sb.Append("  (no context)\n");
    ResumeThread(ht);
    CloseHandle(ht);
    return sb.ToString();
  }
  static IntPtr SymFunctionTableAccess64Wrap(IntPtr p, ulong a) { return SymFunctionTableAccess64(p, a); }
  static ulong SymGetModuleBase64Wrap(IntPtr p, ulong a) { return SymGetModuleBase64(p, a); }

  static IntPtr Proc;
  public static bool Init(int pid, string path) {
    Proc = OpenProcess(0x1F0FFF, false, pid);
    IntPtr hp = Proc;
    SymSetOptions(0x2 | 0x10 | 0x80000000 /*undecorate|load lines|debug*/ | 0x4);
    return SymInitialize(hp, path, true);
  }
}
"@

$p = Get-Process -Id $ProcId
$dir = Split-Path $p.Path
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("process $ProcId $($p.ProcessName) cpu=$([int]$p.TotalProcessorTime.TotalSeconds)s threads=$($p.Threads.Count) working set $([int]($p.WorkingSet64/1MB)) MB")
$ok = [Dump]::Init($ProcId, $dir)
$lines.Add("symbols loaded: $ok")
foreach ($t in $p.Threads | Sort-Object Id) {
  $wait = if ($t.ThreadState -eq "Wait") { $t.WaitReason } else { "" }
  $lines.Add("--- thread $($t.Id) $($t.ThreadState) $wait cpu=$([math]::Round($t.TotalProcessorTime.TotalSeconds,2))s")
  $lines.Add([Dump]::Stack($ProcId, [uint32]$t.Id, $dir))
}
if ($Out) { $lines | Out-File -Encoding utf8 $Out } else { $lines }
