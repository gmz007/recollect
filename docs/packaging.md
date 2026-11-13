# Packaging notes

Recollect is not packaged yet. These notes collect what a distributable build needs.

## Files that must ship together

`cargo build --release` produces `target/release/recollect` and, next to it, the ONNX Runtime
libraries copied by the `ort` crate:

| Platform | Files next to the executable |
|---|---|
| Linux | `libonnxruntime*.so*`, `libwebgpu_dawn.so` |
| Windows | `onnxruntime*.dll`, `webgpu_dawn.dll` |
| macOS | `libonnxruntime*.dylib`, `libwebgpu_dawn.dylib` |

The binary finds them through an rpath (`$ORIGIN` / `@loader_path`, set in `build.rs`); on
Windows the executable's directory is searched by default. Keep them in the same directory.

FFmpeg is linked dynamically. A package must either depend on the system FFmpeg (Linux
distributions) or bundle the FFmpeg shared libraries (Windows, macOS). **Use an LGPL build of
FFmpeg for distributed binaries** (e.g. BtbN's `*-lgpl-shared` builds on Windows); GPL builds
would make the whole distributed package GPL.

## Licenses to include

- Recollect: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- ONNX Runtime: MIT. Dawn (WebGPU): BSD-3-Clause.
- FFmpeg: LGPL-2.1+ (with its source offer / link to sources).
- Rust dependencies: generate a third-party notice file, e.g. with `cargo about`.
- Models are **not** shipped; they are downloaded by the user from Hugging Face on first run
  (CLIP: MIT, SigLIP: Apache-2.0).

## Per-platform ideas (not done)

- Linux: AppImage or Flatpak (Flatpak provides FFmpeg via the `org.freedesktop.Platform.ffmpeg`
  extension).
- Windows: MSI via `cargo-wix`, with the DLLs in the install directory; consider a GUI
  subsystem build (`#![windows_subsystem = "windows"]` for the GUI entry) so no console opens.
- macOS: `.app` bundle with the dylibs in `Contents/Frameworks` (adjust rpath), signed and
  notarized.

## Data locations

See the README table. Uninstalling should leave the user's data directory alone unless the user
asks to remove it (it contains the library database and the downloaded models).
