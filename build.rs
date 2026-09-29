fn main() {
    println!("cargo:rerun-if-env-changed=TELEDRA_BUILD_ID");
    let build_id = std::env::var("TELEDRA_BUILD_ID").unwrap_or_else(|_| "local".into());
    println!("cargo:rustc-env=TELEDRA_BUILD_ID={build_id}");
}
