//! Generates the application icon and embeds it (plus version info) into the
//! Windows executable.

#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out_dir.join("app.ico");
    std::fs::write(&ico_path, icon::ico(&[16, 20, 24, 32, 40, 48, 64, 256]))
        .expect("write app.ico");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().unwrap())
        .set("FileDescription", "Disk Flashlight - disk space analyzer")
        .set("ProductName", "Disk Flashlight");
    if let Err(e) = res.compile() {
        // A missing resource compiler must not break the build.
        println!("cargo:warning=icon not embedded: {e}");
    }
}
