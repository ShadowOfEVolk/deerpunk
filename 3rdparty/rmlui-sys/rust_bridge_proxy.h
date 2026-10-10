/**
 * C++ proxy — форвард в Rust. Без логики.
 */
#ifndef RMLUI_SYS_RUST_BRIDGE_PROXY_H
#define RMLUI_SYS_RUST_BRIDGE_PROXY_H

#include "rust_bridge.h"
#include <RmlUi/Core/FileInterface.h>
#include <RmlUi/Core/RenderInterface.h>
#include <RmlUi/Core/SystemInterface.h>
#include <RmlUi/Core/Types.h>
#include <RmlUi/Core/Vertex.h>

class RustRenderProxy : public Rml::RenderInterface {
    rust_render_handle_t h;
public:
    RustRenderProxy(const rmlui_render_api_t* render_api, const rmlui_callbacks_t* cb)
        : h(rust_create_renderer(render_api, cb)) {}
    ~RustRenderProxy() override { rust_destroy_renderer(h); }
    void SetViewport(int w, int height) { rust_render_set_viewport(h, w, height); }
    void BeginFrame() { rust_render_begin_frame(h); }
    void EndFrame() { rust_render_end_frame(h); }

    Rml::CompiledGeometryHandle CompileGeometry(Rml::Span<const Rml::Vertex> vertices, Rml::Span<const int> indices) override;
    void ReleaseGeometry(Rml::CompiledGeometryHandle geometry) override;
    void RenderGeometry(Rml::CompiledGeometryHandle handle, Rml::Vector2f translation, Rml::TextureHandle texture) override;
    Rml::TextureHandle LoadTexture(Rml::Vector2i& dims, const Rml::String& source) override;
    Rml::TextureHandle GenerateTexture(Rml::Span<const Rml::byte> source, Rml::Vector2i dims) override;
    void ReleaseTexture(Rml::TextureHandle tex) override;
    void EnableScissorRegion(bool enable) override;
    void SetScissorRegion(Rml::Rectanglei r) override;
};

class RustSystemProxy : public Rml::SystemInterface {
    rust_system_handle_t h;
public:
    RustSystemProxy(const rmlui_callbacks_t* cb) : h(rust_create_system(cb)) {}
    ~RustSystemProxy() override { rust_destroy_system(h); }
    double GetElapsedTime() override { return rust_system_get_elapsed_time(h); }
    bool LogMessage(Rml::Log::Type type, const Rml::String& msg) override;
};

class RustFileProxy : public Rml::FileInterface {
public:
    Rml::FileHandle Open(const Rml::String& path) override;
    void Close(Rml::FileHandle file) override;
    size_t Read(void* buffer, size_t size, Rml::FileHandle file) override;
    bool Seek(Rml::FileHandle file, long offset, int origin) override;
    size_t Tell(Rml::FileHandle file) override;
};

#endif
