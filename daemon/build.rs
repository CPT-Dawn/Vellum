use std::path::PathBuf;

use waybackend_scanner::WaylandProtocol;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::var_os("OUT_DIR").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing OUT_DIR environment variable",
        )
    })?;
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../protocols/wlr-layer-shell-unstable-v1.xml");
    println!("cargo:rerun-if-changed=../protocols");

    let mut filepath = PathBuf::from(out_dir);
    filepath.push("wayland_protocols.rs");
    let file = std::fs::File::create(filepath)?;

    waybackend_scanner::build_script_generate(
        &[
            WaylandProtocol::Client,
            WaylandProtocol::System(PathBuf::from_iter(&[
                "stable",
                "viewporter",
                "viewporter.xml",
            ])),
            WaylandProtocol::System(PathBuf::from_iter(&[
                "staging",
                "fractional-scale",
                "fractional-scale-v1.xml",
            ])),
            WaylandProtocol::Local(PathBuf::from_iter(&[
                "../",
                "protocols",
                "wlr-layer-shell-unstable-v1.xml",
            ])),
        ],
        &file,
    );

    Ok(())
}
