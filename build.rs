//! Embeds the app icon in the Windows executable, so Explorer, shortcuts
//! and the pinned taskbar entry show it. Other platforms need nothing here.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource
            .compile()
            .expect("cannot embed assets/icon.ico in the executable");
    }
}
