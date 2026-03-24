fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR");

    pkg_config::Config::new()
        .atleast_version("1.8")
        .probe("liblz4")
        .unwrap();
}
