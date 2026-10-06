//! Embeds Windows resources (the application icon) into the executable.

const ICON: &str = "assets/icons/kegon/kegon.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");

    // Check the target, not the host, so cross-compiling keeps working.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(ICON);
        resource
            .compile()
            .expect("failed to embed Windows resources");
    }
}
