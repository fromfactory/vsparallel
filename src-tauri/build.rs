fn main() {
    for icon in [
        "icons/icon.png",
        "icons/icon.ico",
        "icons/icon.icns",
        "icons/tray-icon-linux.png",
        "icons/tray-icon-macos.png",
        "icons/tray-icon-windows.png",
    ] {
        println!("cargo:rerun-if-changed={icon}");
    }
    tauri_build::build()
}
