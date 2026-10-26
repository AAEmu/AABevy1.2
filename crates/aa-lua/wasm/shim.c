/* Browser build only (see build.rs): Lua's few libc entry points that would pull in WASI libc's
 * number scanning, which clashes with the copy sqlite-wasm-rs links (same symbol names, different
 * long double ABI). Lua's sources are compiled with sprintf/strtod/strtoul/fscanf renamed to these.
 * strtod and strtoul are in Rust (src/wasm_libc.rs). */
#include <stdarg.h>
#include <stdio.h>

/* Every sprintf in Lua 5.1 writes into a buffer of at most LUAL_BUFFERSIZE (512) bytes. */
int aa_lua_sprintf(char *s, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  FILE *f = fmemopen(s, 512, "w");
  int n = -1;
  if (f) {
    n = vfprintf(f, fmt, ap);
    fputc(0, f);
    fclose(f);
  }
  va_end(ap);
  return n;
}

/* io.read("*n"): there are no readable files in the browser. */
int aa_lua_fscanf(FILE *f, const char *fmt, ...) {
  (void)f;
  (void)fmt;
  return 0;
}
