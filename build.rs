use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // FMOD runtime DLLs (fmod.dll / fmodstudio.dll) are loaded manually via
    // LoadLibrary from `{gamedir}/cl_dlls` before the first libfmod call
    // (crates/engine-client-fmod). Delay-load so the exe can start without them.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        println!("cargo:rustc-link-arg=/DELAYLOAD:fmod.dll");
        println!("cargo:rustc-link-arg=/DELAYLOAD:fmodstudio.dll");
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
}
