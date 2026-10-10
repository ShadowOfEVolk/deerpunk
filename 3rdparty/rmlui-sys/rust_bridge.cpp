#include "rust_bridge_proxy.h"
#include <cstring>
#include <vector>

struct GeometryData {
    std::vector<Rml::Vertex> vertices;
    std::vector<int> indices;
};

Rml::CompiledGeometryHandle RustRenderProxy::CompileGeometry(Rml::Span<const Rml::Vertex> vertices, Rml::Span<const int> indices) {
    auto* data = new GeometryData{};
    data->vertices.assign(vertices.begin(), vertices.end());
    data->indices.assign(indices.begin(), indices.end());
    return reinterpret_cast<Rml::CompiledGeometryHandle>(data);
}

void RustRenderProxy::ReleaseGeometry(Rml::CompiledGeometryHandle geometry) {
    delete reinterpret_cast<GeometryData*>(geometry);
}

void RustRenderProxy::RenderGeometry(Rml::CompiledGeometryHandle handle, Rml::Vector2f translation, Rml::TextureHandle texture) {
    const auto* g = reinterpret_cast<const GeometryData*>(handle);
    const Rml::Vertex* v = g->vertices.data();
    const int* idx = g->indices.data();
    const size_t n = g->vertices.size();
    const size_t ni = g->indices.size();

    static constexpr size_t MAX_V = 4096;
    float positions[MAX_V * 2];
    unsigned char colours[MAX_V * 4];
    float texcoords[MAX_V * 2];
    const size_t copy_n = n < MAX_V ? n : MAX_V;

    for (size_t i = 0; i < copy_n; i++) {
        positions[i * 2] = v[i].position.x;
        positions[i * 2 + 1] = v[i].position.y;
        colours[i * 4] = v[i].colour[0];
        colours[i * 4 + 1] = v[i].colour[1];
        colours[i * 4 + 2] = v[i].colour[2];
        colours[i * 4 + 3] = v[i].colour[3];
        texcoords[i * 2] = v[i].tex_coord.x;
        texcoords[i * 2 + 1] = v[i].tex_coord.y;
    }
    rust_render_geometry(h, positions, colours, texcoords, idx, (int)ni, translation.x, translation.y, (uintptr_t)texture);
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
