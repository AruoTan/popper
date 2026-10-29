fn main() {
    // Direct Cargo commands use the same resolver as the Tauri CLI wrapper.
    println!("cargo:rerun-if-changed=../scripts/version.mjs");
    let output = std::process::Command::new("node")
        .args(["../scripts/version.mjs", "--write", "--cargo"])
        .output()
        .expect("Node.js is required to derive the application version from Git tags");
    assert!(
        output.status.success(),
        "Version resolution failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    print!(
        "{}",
        String::from_utf8(output.stdout).expect("UTF-8 version metadata")
    );

    // Tauri embeds the Windows icon in the executable resource table and a
    // decoded copy in generate_context!. Track the sources themselves: the
    // generated files alone do not invalidate a cached build after icon edits.
    println!("cargo:rerun-if-changed=../apps/windows/icons/icon.ico");
    println!("cargo:rerun-if-changed=../apps/windows/icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/icon.png");

    println!("cargo:rerun-if-changed=../apps/macos/native/selection_bridge.h");
    println!("cargo:rerun-if-changed=../apps/macos/native/selection_bridge.mm");
    println!("cargo:rerun-if-changed=../apps/macos/native/LICENSE.selection-hook");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .cpp(true)
            .file("../apps/macos/native/selection_bridge.mm")
            .include("../apps/macos/native")
            .flag_if_supported("-std=c++17")
            .flag_if_supported("-fobjc-arc")
            .warnings(true)
            .compile("popper_selection_bridge");

        for framework in [
            "AppKit",
            "ApplicationServices",
            "Carbon",
            "CoreGraphics",
            "Foundation",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    tauri_build::build()
}
