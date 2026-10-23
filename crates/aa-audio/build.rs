fn main() {
    let dir = "vendor/celt-0.11.0";
    println!("cargo:rerun-if-changed={dir}");
    let mut build = cc::Build::new();
    build.include(dir).std("c11").opt_level(3).warnings(false);
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
