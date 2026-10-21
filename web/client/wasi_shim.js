// Minimal WASI preview1 for libc pieces linked into the browser build: clock, stdout/stderr to
// the console; files and environment report "unsupported".
let memory = null;
export function setMemory(m) { memory = m; }
const view = () => new DataView(memory.buffer);
const EBADF = 8, ENOSYS = 52;
let line = '';
export function fd_write(fd, iovs, iovsLen, nwritten) {
  const v = view(); let total = 0;
  for (let i = 0; i < iovsLen; i++) {
    const p = v.getUint32(iovs + i * 8, true), n = v.getUint32(iovs + i * 8 + 4, true);
    line += new TextDecoder().decode(new Uint8Array(memory.buffer, p, n)); total += n;
  }
  const parts = line.split('\n'); line = parts.pop();
  for (const l of parts) (fd === 2 ? console.warn : console.log)(l);
  v.setUint32(nwritten, total, true); return 0;
}
export function clock_time_get(id, precision, out) {
  const ns = id === 0 ? BigInt(Date.now()) * 1000000n : BigInt(Math.round(performance.now() * 1e6));
  view().setBigUint64(out, ns, true); return 0;
}
export function environ_sizes_get(count, size) { const v = view(); v.setUint32(count, 0, true); v.setUint32(size, 0, true); return 0; }
export function environ_get() { return 0; }
export function proc_exit(code) { throw new Error(`proc_exit(${code})`); }
export function fd_prestat_get() { return EBADF; }
export function fd_prestat_dir_name() { return EBADF; }
export function fd_close() { return 0; }
export function fd_fdstat_get() { return EBADF; }
export function fd_fdstat_set_flags() { return ENOSYS; }
export function fd_read() { return EBADF; }
export function fd_renumber() { return ENOSYS; }
export function fd_seek() { return EBADF; }
export function path_open() { return ENOSYS; }
export function path_remove_directory() { return ENOSYS; }
export function path_rename() { return ENOSYS; }
export function path_unlink_file() { return ENOSYS; }
export function random_get(buf, len) { crypto.getRandomValues(new Uint8Array(memory.buffer, buf, len)); return 0; }
