fn main() {
    // Embed the application icon into the Windows executable so Explorer, the
    // taskbar, and the window itself show it. Pure no-op on other targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            // Version resource: FileDescription is the name Windows shows in
            // the "Open with…" list (and Explorer's file-properties dialog);
            // without it the shell falls back to the raw exe name.
            .set("FileDescription", "SoftOxide JXL Viewer")
            .set("ProductName", "SoftOxide JXL Viewer")
            .set("CompanyName", "SoftOxide")
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("OriginalFilename", "jxlviewer.exe")
            .set("LegalCopyright", "Dual-licensed under MIT OR Apache-2.0")
            .compile()
            .expect("failed to compile Windows resources (icon embedding)");
    }
}
