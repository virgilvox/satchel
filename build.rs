// Embed the SATCHEL mark as the executable icon on Windows. macOS shows
// the icon only when wrapped in a `.app` bundle (out of scope for the raw
// CLI binary; the .icns ships alongside in the release zip). Linux ELF
// binaries have no icon resource — ship the PNG and let .desktop files
// reference it.

fn main() {
    println!("cargo:rerun-if-changed=assets/brand/satchel.ico");
    println!("cargo:rerun-if-changed=build.rs");

    // CARGO_CFG_TARGET_OS reflects the build TARGET, not the host. This is
    // the only correct way to gate the Windows-resource step from a build
    // script when cross-compiling.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        embed_windows_icon();
    }
}

#[cfg(target_os = "windows")]
fn embed_windows_icon() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/brand/satchel.ico");
    if let Err(e) = res.compile() {
        // Don't fail the whole build over a cosmetic icon — print a hint
        // and carry on so non-MSVC Windows toolchains still link.
        println!("cargo:warning=failed to embed icon: {e}");
    }
}

// On non-Windows hosts the `winresource` dep isn't pulled in (target-gated
// in Cargo.toml), so the import would fail at link. Provide a no-op stub
// for cross-compilation builds (Linux/macOS host → Windows target).
#[cfg(not(target_os = "windows"))]
fn embed_windows_icon() {
    println!(
        "cargo:warning=Windows target detected on a non-Windows host; \
         skipping icon embed (winresource is not available cross-platform)."
    );
}
