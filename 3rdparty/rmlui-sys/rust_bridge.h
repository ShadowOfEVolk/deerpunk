/**
 * rust_bridge.h — C API для вызова Rust-реализации Render/System/File.
 * Rust (menu crate) реализует эти функции, используя triapi/ref из Xash.
 */
#ifndef RMLUI_SYS_RUST_BRIDGE_H
#define RMLUI_SYS_RUST_BRIDGE_H

#include "api.h"
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* rust_render_handle_t;
typedef void* rust_system_handle_t;
typedef void* rust_file_handle_t;

rust_render_handle_t rust_create_renderer(const rmlui_render_api_t* render_api, const rmlui_callbacks_t* cb);
void rust_destroy_renderer(rust_render_handle_t handle);
void rust_render_set_viewport(rust_render_handle_t handle, int width, int height);
void rust_render_begin_frame(rust_render_handle_t h);
void rust_render_end_frame(rust_render_handle_t h);

void rust_render_geometry(rust_render_handle_t handle,
    const float* positions, const unsigned char* colours, const float* texcoords,
    const int* indices, int num_indices,
    float tx, float ty, uintptr_t texture);

uintptr_t rust_load_texture(rust_render_handle_t handle, const char* source, int* out_w, int* out_h);
uintptr_t rust_generate_texture(rust_render_handle_t handle, const unsigned char* rgba, int width, int height);
void rust_release_texture(rust_render_handle_t handle, uintptr_t tex);

void rust_scissor_enable(rust_render_handle_t handle, int enable);
void rust_scissor_region(rust_render_handle_t handle, int x, int y, int w, int h);

rust_system_handle_t rust_create_system(const rmlui_callbacks_t* cb);
void rust_destroy_system(rust_system_handle_t handle);
double rust_system_get_elapsed_time(rust_system_handle_t handle);
int rust_system_log_message(rust_system_handle_t handle, int level, const char* msg);

rust_file_handle_t rust_file_open(const char* path);
void rust_file_close(rust_file_handle_t file);
size_t rust_file_read(rust_file_handle_t file, void* buf, size_t size);
int rust_file_seek(rust_file_handle_t file, long offset, int origin);
size_t rust_file_tell(rust_file_handle_t file);

#ifdef __cplusplus
}
#endif

#endif
