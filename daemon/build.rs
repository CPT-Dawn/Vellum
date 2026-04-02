use std::{
    io::{BufWriter, Write},
    path::PathBuf,
};

use waybackend_scanner::Protocol;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let out_dir = std::env::var_os("OUT_DIR").expect("missing OUT_DIR environment variable");

    let mut filepath = PathBuf::from(out_dir);
    filepath.push("wayland_protocols.rs");
    let file = std::fs::File::create(filepath).expect("failed to create wayland_protocols.rs");
    let mut writer = BufWriter::new(file);

    let protocol_paths = [
        manifest_dir.join("protocols/wayland.xml"),
        manifest_dir.join("protocols/stable/viewporter/viewporter.xml"),
        manifest_dir.join("protocols/staging/fractional-scale/fractional-scale-v1.xml"),
        manifest_dir.join("../protocols/wlr-layer-shell-unstable-v1.xml"),
    ];

    for protocol_path in protocol_paths {
        if !protocol_path.is_file() {
            panic!("missing wayland protocol file: {}", protocol_path.display());
        }

        println!("cargo:rerun-if-changed={}", protocol_path.display());
        let code = Protocol::new(&protocol_path).generate();
        writeln!(writer, "{code}").expect("failed to write generated protocol code");
    }
}
