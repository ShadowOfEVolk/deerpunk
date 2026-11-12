#include "rust_bridge_proxy.h"
#include <cstddef>

static_assert(offsetof(Rml::Vertex, position) == 0, "Rml vertex position offset");
static_assert(offsetof(Rml::Vertex, colour) == 8, "Rml vertex colour offset");
static_assert(offsetof(Rml::Vertex, tex_coord) == 12, "Rml vertex uv offset");

Rml::CompiledGeometryHandle RustRenderProxy::CompileGeometry(Rml::Span<const Rml::Vertex> vertices, Rml::Span<const int> indices) {
    const uint32_t id = rust_compile_geometry(
        vertices.data(),
        (int)sizeof(Rml::Vertex),
        (int)vertices.size(),
        indices.data(),
        (int)indices.size());
    return static_cast<Rml::CompiledGeometryHandle>(id);
}

void RustRenderProxy::ReleaseGeometry(Rml::CompiledGeometryHandle geometry) {
    rust_release_geometry(static_cast<uint32_t>(geometry));
}

void RustRenderProxy::RenderGeometry(Rml::CompiledGeometryHandle handle, Rml::Vector2f translation, Rml::TextureHandle texture) {
    rust_draw_geometry(
        static_cast<uint32_t>(handle),
        translation.x,
        translation.y,
        (uintptr_t)texture);
}

Rml::TextureHandle RustRenderProxy::LoadTexture(Rml::Vector2i& dims, const Rml::String& source) {
    int w = 0, height = 0;
    uintptr_t ret = rust_load_texture(h, source.c_str(), &w, &height);
    dims.x = w;
    dims.y = height;
    return (Rml::TextureHandle)ret;
}

Rml::TextureHandle RustRenderProxy::GenerateTexture(Rml::Span<const Rml::byte> source, Rml::Vector2i dims) {
    return (Rml::TextureHandle)rust_generate_texture(h, source.data(), dims.x, dims.y);
}

void RustRenderProxy::ReleaseTexture(Rml::TextureHandle tex) {
    rust_release_texture(h, (uintptr_t)tex);
}

void RustRenderProxy::EnableScissorRegion(bool enable) {
    rust_scissor_enable(h, enable ? 1 : 0);
}

void RustRenderProxy::SetScissorRegion(Rml::Rectanglei rect) {
    rust_scissor_region(h, rect.Left(), rect.Top(), rect.Width(), rect.Height());
}

bool RustSystemProxy::LogMessage(Rml::Log::Type type, const Rml::String& msg) {
    int level = 2;
    if (type == Rml::Log::LT_ERROR) level = 0;
    else if (type == Rml::Log::LT_WARNING) level = 1;
    return rust_system_log_message(h, level, msg.c_str()) != 0;
}

Rml::FileHandle RustFileProxy::Open(const Rml::String& path) {
    return (Rml::FileHandle)rust_file_open(path.c_str());
}

void RustFileProxy::Close(Rml::FileHandle file) {
    rust_file_close((rust_file_handle_t)file);
}

size_t RustFileProxy::Read(void* buffer, size_t size, Rml::FileHandle file) {
    return rust_file_read((rust_file_handle_t)file, buffer, size);
}

bool RustFileProxy::Seek(Rml::FileHandle file, long offset, int origin) {
    return rust_file_seek((rust_file_handle_t)file, offset, origin) == 0;
}

size_t RustFileProxy::Tell(Rml::FileHandle file) {
    return rust_file_tell((rust_file_handle_t)file);
}
