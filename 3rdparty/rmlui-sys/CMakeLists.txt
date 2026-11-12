cmake_minimum_required(VERSION 3.12)
project(rmlui-sys CXX C)

set(CMAKE_CXX_STANDARD 17)

find_package(OpenGL REQUIRED)

# Override with -DDEERPUNK_3RDPARTY=... when 3rdparty entries are cross-drive junctions
# (MSVC often fails to open sources through \\?\ + junction to another drive).
if(NOT DEFINED DEERPUNK_3RDPARTY)
    set(DEERPUNK_3RDPARTY "${CMAKE_CURRENT_SOURCE_DIR}/..")
endif()

set(FREETYPE_DIR "${DEERPUNK_3RDPARTY}/freetype")
if(NOT EXISTS "${FREETYPE_DIR}/CMakeLists.txt")
    message(FATAL_ERROR "FreeType not found at ${FREETYPE_DIR}")
endif()
set(FT_DISABLE_ZLIB TRUE CACHE BOOL "" FORCE)
set(FT_DISABLE_BZIP2 TRUE CACHE BOOL "" FORCE)
set(FT_DISABLE_PNG TRUE CACHE BOOL "" FORCE)
set(FT_DISABLE_HARFBUZZ TRUE CACHE BOOL "" FORCE)
set(FT_DISABLE_BROTLI TRUE CACHE BOOL "" FORCE)
# Apple clang ships HVF headers; without linking Apple's HVF runtime the static freetype
# library leaves undefined HVF_* symbols at final link.
set(FT_DISABLE_HVF TRUE CACHE BOOL "" FORCE)
add_subdirectory("${FREETYPE_DIR}" "${CMAKE_BINARY_DIR}/freetype" EXCLUDE_FROM_ALL)
add_library(Freetype::Freetype ALIAS freetype)

set(LUA_DIR "${DEERPUNK_3RDPARTY}/lua-src")
if(NOT EXISTS "${LUA_DIR}/lapi.c")
    message(FATAL_ERROR "Lua sources not found at ${LUA_DIR} (expected lua-src submodule)")
endif()

# Upstream lua/lua ships no CMake build. Define the static library here from
# the stock Makefile's CORE_O + AUX_O + LIB_O source lists (minus ltests.o) —
# a hand-written CMakeLists.txt inside the submodule would stay untracked and
# never reach fresh clones/CI.
add_library(lua STATIC
    ${LUA_DIR}/lapi.c
    ${LUA_DIR}/lcode.c
    ${LUA_DIR}/lctype.c
    ${LUA_DIR}/ldebug.c
    ${LUA_DIR}/ldo.c
    ${LUA_DIR}/ldump.c
    ${LUA_DIR}/lfunc.c
    ${LUA_DIR}/lgc.c
    ${LUA_DIR}/llex.c
    ${LUA_DIR}/lmem.c
    ${LUA_DIR}/lobject.c
    ${LUA_DIR}/lopcodes.c
    ${LUA_DIR}/lparser.c
    ${LUA_DIR}/lstate.c
    ${LUA_DIR}/lstring.c
    ${LUA_DIR}/ltable.c
    ${LUA_DIR}/ltm.c
    ${LUA_DIR}/lundump.c
    ${LUA_DIR}/lvm.c
    ${LUA_DIR}/lzio.c
    ${LUA_DIR}/lauxlib.c
    ${LUA_DIR}/lbaselib.c
    ${LUA_DIR}/ldblib.c
    ${LUA_DIR}/liolib.c
    ${LUA_DIR}/lmathlib.c
    ${LUA_DIR}/loslib.c
    ${LUA_DIR}/ltablib.c
    ${LUA_DIR}/lstrlib.c
    ${LUA_DIR}/lutf8lib.c
    ${LUA_DIR}/loadlib.c
    ${LUA_DIR}/lcorolib.c
    ${LUA_DIR}/linit.c
)
target_include_directories(lua PUBLIC ${LUA_DIR})

if(APPLE)
    target_compile_definitions(lua PRIVATE LUA_USE_MACOSX)
elseif(UNIX)
    target_compile_definitions(lua PRIVATE LUA_USE_LINUX)
endif()

target_link_libraries(lua PUBLIC m)

add_library(Lua::Lua INTERFACE IMPORTED)
set_target_properties(Lua::Lua PROPERTIES
    INTERFACE_LINK_LIBRARIES lua
    INTERFACE_INCLUDE_DIRECTORIES "${LUA_DIR}"
)

set(RMLUI_DIR "${DEERPUNK_3RDPARTY}/RmlUi")
if(NOT EXISTS "${RMLUI_DIR}/CMakeLists.txt")
    message(FATAL_ERROR "RmlUi not found at ${RMLUI_DIR}")
endif()

set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(RMLUI_SAMPLES OFF CACHE BOOL "" FORCE)
set(RMLUI_TESTS OFF CACHE BOOL "" FORCE)
set(RMLUI_FONT_ENGINE "freetype" CACHE STRING "" FORCE)
set(RMLUI_LUA_BINDINGS ON CACHE BOOL "" FORCE)
set(RMLUI_LUA_BINDINGS_LIBRARY "lua" CACHE STRING "" FORCE)
add_subdirectory("${RMLUI_DIR}" "${CMAKE_BINARY_DIR}/RmlUi" EXCLUDE_FROM_ALL)

add_library(rmlui_sys STATIC
    api.cpp
    rust_bridge.cpp
)

target_include_directories(rmlui_sys
    PUBLIC
        ${CMAKE_CURRENT_SOURCE_DIR}
        "${RMLUI_DIR}/Include"
)

target_link_libraries(rmlui_sys
    PUBLIC
        rmlui_core
        rmlui_lua
    PRIVATE
        OpenGL::GL
)
