// Cargo reads build-script instructions from stdout.
#![allow(clippy::print_stdout)]

fn main() {
    println!("cargo:rerun-if-changed=dusk.rc");
    println!("cargo:rerun-if-changed=../../assets/icons/dusk.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("dusk.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the app icon");
    }
}
