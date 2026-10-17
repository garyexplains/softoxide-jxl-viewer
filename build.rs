fn main() {
    // Embed the application icon into the Windows executable so Explorer, the
    // taskbar, and the window itself show it. Pure no-op on other targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("failed to compile Windows resources (icon embedding)");
    }
}
