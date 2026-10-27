fn main() {
    let dir = "vendor/celt-0.11.0";
    println!("cargo:rerun-if-changed={dir}");
    let mut build = cc::Build::new();
    build.include(dir).std("c11").opt_level(3).warnings(false);
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rerun-if-env-changed=WASI_SDK");
        let sdk = std::env::var("WASI_SDK").expect("the browser build needs WASI_SDK: the WASI SDK folder");
        let exe = if cfg!(windows) { ".exe" } else { "" };
        let sysroot = format!("{sdk}/share/wasi-sysroot");
        // Match the browser Lua build: the legacy C decoder needs libc headers too.
        build.compiler(format!("{sdk}/bin/clang{exe}"))
            .archiver(format!("{sdk}/bin/llvm-ar{exe}"))
            .target("wasm32-wasip1")
            .flag("--target=wasm32-wasip1")
            .flag(format!("--sysroot={sysroot}"));
        println!("cargo:rustc-link-search=native={sysroot}/lib/wasm32-wasip1");
        println!("cargo:rustc-link-lib=static=c");
    }
    for define in [
        "AA_CELT_STATIC",
        "AA_CELT_DECODER_ONLY",
        "FLOATING_POINT",
        "CUSTOM_MODES",
        "CUSTOM_MODES_ONLY",
        "USE_ALLOCA",
    ] {
        build.define(define, None);
    }
    if build.get_compiler().is_like_msvc() {
        build.define("WIN32", None).define("restrict", "__restrict");
        build.flag("/fp:precise");
    } else {
        build
            .define("HAVE_ALLOCA_H", None)
            .define("HAVE_LRINTF", "1");
        build.flag_if_supported("-ffp-contract=off");
    }
    for file in [
        "aa_bridge",
        "bands",
        "celt",
        "cwrs",
        "entcode",
        "entdec",
        "entenc",
        "kiss_fft",
        "laplace",
        "mathops",
        "mdct",
        "modes",
        "pitch",
        "plc",
        "quant_bands",
        "rate",
        "vq",
    ] {
        build.file(format!("{dir}/{file}.c"));
    }
    build.compile("aa_celt011");
}
