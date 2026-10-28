use std::env;
use std::path::{Path, PathBuf};

/// MSVC fails to open sources under `\\?\` extended paths from cmake/cargo.
fn strip_extended_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn abs(path: &Path) -> PathBuf {
    strip_extended_path(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let rmlui_sys_dir = abs(&manifest_dir.join("../../3rdparty/rmlui-sys"));

    println!("cargo:rerun-if-changed={}", rmlui_sys_dir.join("api.h").display());
    println!("cargo:rerun-if-changed={}", rmlui_sys_dir.join("api.cpp").display());
    println!("cargo:rerun-if-changed={}", rmlui_sys_dir.join("rust_bridge.cpp").display());

    let deps = env::var("DEERPUNK_3RDPARTY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| abs(&manifest_dir.join("../../3rdparty")));
    if !deps.join("RmlUi").join("CMakeLists.txt").exists() {
        panic!(
            "RmlUi deps not found at {}. Expected submodules RmlUi, freetype, lua-src under 3rdparty/.",
            deps.display()
        );
    }

    let mut cmake = cmake::Config::new(&rmlui_sys_dir);
    cmake.define(
        "CMAKE_BUILD_TYPE",
        if cfg!(debug_assertions) {
            "Debug"
        } else {
            "Release"
        },
    );
    cmake.define("DEERPUNK_3RDPARTY", deps.to_str().unwrap());
    cmake.cxxflag("/EHsc");

    let dst = cmake.build();
    let build_dir = dst.join("build");

    let search_dirs = [
        build_dir.join("Debug"),
        build_dir.join("Release"),
        build_dir.clone(),
        dst.join("lib"),
        dst.clone(),
        build_dir.join("RmlUi").join("Source").join("Core").join("Debug"),
        build_dir.join("RmlUi").join("Source").join("Core").join("Release"),
        build_dir.join("RmlUi").join("Source").join("Core"),
        build_dir.join("RmlUi").join("Source").join("Lua").join("Debug"),
        build_dir.join("RmlUi").join("Source").join("Lua").join("Release"),
        build_dir.join("_deps").join("freetype-build").join("Debug"),
        build_dir.join("_deps").join("freetype-build").join("Release"),
        build_dir.join("freetype").join("Debug"),
        build_dir.join("freetype").join("Release"),
        build_dir.join("lua").join("Debug"),
        build_dir.join("lua").join("Release"),
    ];

    for lib_dir in &search_dirs {
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
        }
    }

    println!("cargo:rustc-link-lib=static=rmlui_sys");
    println!("cargo:rustc-link-lib=static=rmlui");
    println!("cargo:rustc-link-lib=static=rmlui_lua");
    println!("cargo:rustc-link-lib=static=lua");
    if cfg!(debug_assertions) {
        println!("cargo:rustc-link-lib=static=freetyped");
    } else {
        println!("cargo:rustc-link-lib=static=freetype");
    }
    println!("cargo:rustc-link-lib=opengl32");
    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=shell32");
    println!("cargo:rustc-link-lib=gdi32");

    // MSVC C++ runtime
    println!("cargo:rustc-link-lib=dylib=msvcrt");

    let bindings = bindgen::Builder::default()
        .header(rmlui_sys_dir.join("api.h").to_str().unwrap())
        .allowlist_function("rmlui_.*")
        .allowlist_type("rmlui_.*")
        .allowlist_var("RMLUI_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen rmlui-sys");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("rmlui_sys.rs"))
        .expect("write rmlui bindings");
}
