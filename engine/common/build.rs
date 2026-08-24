fn main() {
    let flecs_dir = std::path::Path::new("flecs");

    println!("cargo:rerun-if-changed=flecs/flecs.c");
    println!("cargo:rerun-if-changed=flecs/flecs.h");
    println!("cargo:rerun-if-changed=flecs/shim.c");

    cc::Build::new()
        .file(flecs_dir.join("flecs.c"))
        .include(flecs_dir)
        .std("c11")
        .warnings(false)
        .flag_if_supported("/GS-")
        .compile("flecs");

    cc::Build::new()
        .file(flecs_dir.join("shim.c"))
        .include(flecs_dir)
        .std("c11")
        .warnings(false)
        .flag_if_supported("/GS-")
        .compile("flecs_shim");
}
