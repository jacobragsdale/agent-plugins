fn main() {
    #[cfg(feature = "app")]
    {
        let windows = tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("windows-app-manifest.xml"));
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to run the Tauri build script");
    }
    #[cfg(not(feature = "app"))]
    {
        // tauri-build normally declares these cfgs. Without the desktop
        // application they are never set, but the crate still names them.
        println!("cargo:rustc-check-cfg=cfg(desktop)");
        println!("cargo:rustc-check-cfg=cfg(mobile)");
    }
}
