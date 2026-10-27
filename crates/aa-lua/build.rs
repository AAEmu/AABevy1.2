//! Browser build only: compiles Lua 5.1 (`lua51/`, unmodified 5.1.5) for wasm32 with the WASI SDK's
//! clang, because mlua's vendored build does not support `wasm32-unknown-unknown`. Lua's
//! setjmp/longjmp error handling becomes WebAssembly exception handling (`-wasm-enable-sjlj`); the
//! few libc functions it needs come from the WASI sysroot (the page supplies the WASI imports, see
//! `web/client/wasi_shim.js`). Native builds use mlua's vendored Lua and skip all of this.
//!
//! `WASI_SDK` points at the SDK.
fn main() {
    println!("cargo:rerun-if-changed=lua51");
    println!("cargo:rerun-if-changed=wasm");
    println!("cargo:rerun-if-env-changed=WASI_SDK");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        return;
    }
    let sdk = std::env::var("WASI_SDK").expect("the browser build needs WASI_SDK: the WASI SDK folder");
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let sysroot = format!("{sdk}/share/wasi-sysroot");
    let mut b = cc::Build::new();
    b.compiler(format!("{sdk}/bin/clang{exe}"))
        .archiver(format!("{sdk}/bin/llvm-ar{exe}"))
        .target("wasm32-wasip1")
        .flag("--target=wasm32-wasip1")
        .flag(format!("--sysroot={sysroot}"))
        .flag("-mllvm")
        .flag("-wasm-enable-eh")
        .flag("-mllvm")
        .flag("-wasm-use-legacy-eh=false")
        .flag("-mllvm")
        .flag("-wasm-enable-sjlj")
        .define("_WASI_EMULATED_SIGNAL", None)
        .define("_WASI_EMULATED_PROCESS_CLOCKS", None)
        .define("LUA_USE_POSIX", None)
        // WASI libc's number scanner clashes with sqlite-wasm-rs's (see wasm/shim.c)
        .define("sprintf", "aa_lua_sprintf")
        .define("strtod", "aa_lua_strtod")
        .define("strtoul", "aa_lua_strtoul")
        .define("fscanf", "aa_lua_fscanf")
        .warnings(false)
        .opt_level(2);
    for e in std::fs::read_dir("lua51").expect("lua51 sources") {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".c") && !["lua.c", "luac.c", "print.c"].contains(&name.as_str()) {
            b.file(&p);
        }
    }
    b.file("wasm/shim.c");
    b.compile("lua5.1");
    println!("cargo:rustc-link-search=native={sysroot}/lib/wasm32-wasip1");
    for lib in ["setjmp", "c", "wasi-emulated-signal", "wasi-emulated-process-clocks"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
}
