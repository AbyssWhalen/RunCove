fn main() {
    tauri_build::build();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resource = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR to build scripts"),
        )
        .join("resource.rc");

        embed_resource::compile_for_everything(resource, embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed the Windows application manifest in test executables");
    }
}
