//! Generates the Wayland protocol bindings from the vendored XML files.

use std::io::Write;
use std::path::PathBuf;

const PROTOCOLS: &[&str] = &["wayland.xml", "wlr-layer-shell-unstable-v1.xml"];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("protocols.rs");
    let mut file = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    for name in PROTOCOLS {
        let path = manifest_dir.join("protocols").join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let code = waybackend_scanner::Protocol::new(&path).generate();
        writeln!(file, "{code}").unwrap();
    }
}
