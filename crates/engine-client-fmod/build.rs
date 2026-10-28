fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // NOTE: /DELAYLOAD must be applied to the final binary link — it lives in
    // the root `build.rs` (bin `deerpunk`). Link-args from dependency build
    // scripts don't reach the binary link step.
}
