fn main() {
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET to build scripts");
    println!("cargo:rustc-env=STREAMS_TARGET_TRIPLE={target}");
    println!("cargo:rerun-if-env-changed=TARGET");
}
