cmake_minimum_required(VERSION 3.12)
project(rmlui-sys CXX)

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
add_subdirectory("${FREETYPE_DIR}" "${CMAKE_BINARY_DIR}/freetype" EXCLUDE_FROM_ALL)
add_library(Freetype::Freetype ALIAS freetype)

set(LUA_DIR "${DEERPUNK_3RDPARTY}/lua-src")
if(NOT EXISTS "${LUA_DIR}/CMakeLists.txt")
    message(FATAL_ERROR "Lua not found at ${LUA_DIR} (expected lua-src submodule)")
endif()
add_subdirectory("${LUA_DIR}" "${CMAKE_BINARY_DIR}/lua" EXCLUDE_FROM_ALL)

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
