fn main() {
    for file in [
        "duskd.rc",
        "duskd.manifest",
        "../../assets/icons/dusk.ico",
        "../../assets/icons/tray-light.ico",
        "../../assets/icons/tray-dark.ico",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("duskd.rc", embed_resource::NONE)
            .manifest_required()
            .expect("embedding the Windows manifest and icons");
    }
}
