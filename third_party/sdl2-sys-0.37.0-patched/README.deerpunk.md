# sdl2-sys fork (deerpunk)

Vendored copy of crates.io `sdl2-sys` 0.37.0 with a one-line patch in `build.rs`:

- `CMAKE_POLICY_VERSION_MINIMUM=3.5` — allows the bundled SDL2 CMakeLists to configure under **CMake 4.x** (which otherwise errors on `cmake_minimum_required(VERSION 2.x...3.4)`).

The workspace root `[patch.crates-io]` points `sdl2-sys` here. Remove this vendor when upstream `sdl2-sys` ships the same fix.
