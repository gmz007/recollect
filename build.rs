//! Lets the binary find the ONNX Runtime / Dawn shared libraries that `ort` copies next to it.
//! (Windows already searches the executable's directory.)

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let dev = std::env::var("PROFILE").map_or(true, |p| p != "release");
    let origin = match os.as_str() {
        "linux" | "freebsd" => "$ORIGIN",
        "macos" => "@loader_path",
        _ => return,
    };
    println!("cargo:rustc-link-arg=-Wl,-rpath,{origin}");
    if dev {
        // Test binaries live in target/<profile>/deps, one level below the copied libraries.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{origin}/..");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
