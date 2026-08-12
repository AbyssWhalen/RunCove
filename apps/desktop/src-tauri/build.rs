fn main() {
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("failed to run the Tauri build script");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resource_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("windows");
        let resource = resource_dir.join("app-manifest.rc");
        let manifest = resource_dir.join("app-manifest.xml");

        println!("cargo:rerun-if-changed={}", resource.display());
        println!("cargo:rerun-if-changed={}", manifest.display());

        // Tauri links VERSION and icon resources to the application binary.
        // Keep the manifest separate so unit-test executables receive it too.
        embed_resource::compile_for_everything(
            resource,
            embed_resource::ParamsIncludeDirs([resource_dir]),
        )
        .manifest_required()
        .expect("failed to embed the Windows application manifest in test executables");
    }
}
