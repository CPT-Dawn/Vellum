//! Properties of the `vellumd` binary that need no compositor.

use std::process::Command;

/// ADR 0003: the Daemon links no image-decoding or Config-parsing code.
#[test]
fn vellumd_links_no_image_decoders_or_config_parsers() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "--offline",
            "--package",
            "vellumd",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{lib}",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8(out.stdout).unwrap();
    let decoders = [
        "image",
        "png",
        "jpeg_decoder",
        "zune_jpeg",
        "zune_core",
        "image_webp",
        "gif",
        "tiff",
        "ravif",
        "jxl_oxide",
        "dav1d",
        "resvg",
    ];
    // ...and no Config parsing: only `vellum` reads TOML.
    let config_parsers = [
        "toml",
        "toml_parser",
        "toml_edit",
        "serde",
        "serde_core",
        "serde_spanned",
    ];
    for line in tree.lines() {
        let krate = line.trim();
        assert!(
            !decoders.contains(&krate),
            "vellumd depends on the decoder crate `{krate}`:\n{tree}"
        );
        assert!(
            !config_parsers.contains(&krate),
            "vellumd depends on the Config-parsing crate `{krate}` (ADR 0003):\n{tree}"
        );
    }
    assert!(tree.lines().any(|l| l.trim() == "waybackend"), "{tree}");
}

#[test]
fn without_a_compositor_vellumd_fails_with_a_clear_message() {
    let dir = tempfile_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_vellumd"))
        .env_clear()
        .env("XDG_RUNTIME_DIR", &dir)
        .env("WAYLAND_DISPLAY", "wayland-nowhere")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("compositor"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vellumd-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
