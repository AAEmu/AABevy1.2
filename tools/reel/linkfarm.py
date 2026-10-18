"""Hard-link target/debug/{.fingerprint,build,deps} of the shared target dir into another target dir (no disk, so a worktree at
another path can be built once without compiling Bevy again and without touching the shared dir or its lock)."""
import os, sys
src, dst = sys.argv[1], sys.argv[2]
n = 0
for sub in (".fingerprint", "build", "deps"):
    for root, dirs, files in os.walk(os.path.join(src, "debug", sub)):
        rel = os.path.relpath(root, src)
        os.makedirs(os.path.join(dst, rel), exist_ok=True)
        for f in files:
            d = os.path.join(dst, rel, f)
            if not os.path.exists(d):
                try:
                    os.link(os.path.join(root, f), d)
                    n += 1
                except OSError as e:
                    print("skip", f, e)
for f in (".rustc_info.json", "CACHEDIR.TAG"):
    if os.path.exists(os.path.join(src, f)):
        import shutil
        shutil.copy(os.path.join(src, f), os.path.join(dst, f))
print("linked", n)
