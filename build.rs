//! Embeds the game icon and version details into the Windows executable, so
//! it shows the 8-ball icon in Explorer, the taskbar and installers.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "Breakpoint Billiards")
        .set("FileDescription", "Breakpoint Billiards")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        // The game still builds; it just has the default icon.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
