#include "api.h"
#include "rust_bridge_proxy.h"

#include <RmlUi/Core/Core.h>
#include <RmlUi/Core/Context.h>
#include <RmlUi/Core/DataModelHandle.h>
#include <RmlUi/Core/ElementDocument.h>
#include <RmlUi/Core/Factory.h>
#include <RmlUi/Core/Input.h>
#include <RmlUi/Core/Variant.h>
#include <RmlUi/Lua/Lua.h>
#include <RmlUi/Lua/Interpreter.h>
#include <RmlUi/Lua/IncludeLua.h>
#include <memory>

static std::unique_ptr<Rml::RenderInterface> g_renderer;
static const rmlui_callbacks_t* g_callbacks = nullptr;
static std::unique_ptr<Rml::SystemInterface> g_system;
static std::unique_ptr<Rml::FileInterface> g_file;
static int g_viewport_w = 0, g_viewport_h = 0;

static int lua_cmd(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->execute_command) return 0;
    const char* s = luaL_checkstring(L, 1);
    if (s && s[0]) g_callbacks->execute_command(s);
    return 0;
}

static int lua_in_game(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->client_in_game) {
        lua_pushboolean(L, 0);
        return 1;
    }
    lua_pushboolean(L, g_callbacks->client_in_game() != 0);
    return 1;
}

static int lua_cvar_float(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->get_cvar_float) {
        lua_pushnumber(L, 0.0);
        return 1;
    }
    const char* s = luaL_checkstring(L, 1);
    float v = s && s[0] ? g_callbacks->get_cvar_float(s) : 0.0f;
    lua_pushnumber(L, (lua_Number)v);
    return 1;
}

static int lua_cvar_string(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->get_cvar_string) {
        lua_pushstring(L, "");
        return 1;
    }
    const char* s = luaL_checkstring(L, 1);
    const char* v = s && s[0] ? g_callbacks->get_cvar_string(s) : "";
    lua_pushstring(L, v ? v : "");
    return 1;
}

static int lua_in_background(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->get_cvar_float) {
        lua_pushboolean(L, 0);
        return 1;
    }
    float v = g_callbacks->get_cvar_float("cl_background");
    lua_pushboolean(L, v != 0.0f);
    return 1;
}

static int lua_eng_get_files_list(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->get_files_list) {
        lua_pushstring(L, "[]");
        return 1;
    }
    const char* pattern = luaL_checkstring(L, 1);
    const char* json = g_callbacks->get_files_list(pattern);
    lua_pushstring(L, json ? json : "[]");
    return 1;
}

static int lua_eng_load_file(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->load_file) {
        lua_pushnil(L);
        return 1;
    }
    const char* path = luaL_checkstring(L, 1);
    const char* content = g_callbacks->load_file(path);
    if (content && content[0])
        lua_pushstring(L, content);
    else
        lua_pushnil(L);
    return 1;
}

static void register_lua_commands(void)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L) return;
    lua_pushcfunction(L, lua_cmd);
    lua_setglobal(L, "cmd");
    lua_pushcfunction(L, lua_in_game);
    lua_setglobal(L, "in_game");
    lua_pushcfunction(L, lua_cvar_float);
    lua_setglobal(L, "cvar_float");
    lua_pushcfunction(L, lua_cvar_string);
    lua_setglobal(L, "cvar_string");
    lua_pushcfunction(L, lua_in_background);
    lua_setglobal(L, "in_background");
    lua_pushcfunction(L, lua_eng_get_files_list);
    lua_setglobal(L, "eng_get_files_list");
    lua_pushcfunction(L, lua_eng_load_file);
    lua_setglobal(L, "eng_load_file");
}

int rmlui_init(int viewport_w, int viewport_h, const rmlui_callbacks_t* callbacks, const rmlui_render_api_t* render_api)
{
    if (g_renderer) return 1;
    g_callbacks = callbacks;

    if (!render_api || !render_api->TriRenderMode || !render_api->Begin || !render_api->End || !render_api->GL_CreateTexture) {
        return 0;
    }

    g_viewport_w = viewport_w;
    g_viewport_h = viewport_h;

    g_system = std::make_unique<RustSystemProxy>(callbacks);
    g_file = std::make_unique<RustFileProxy>();
    g_renderer = std::make_unique<RustRenderProxy>(render_api, callbacks);

    if (auto* proxy = dynamic_cast<RustRenderProxy*>(g_renderer.get()))
        proxy->SetViewport(viewport_w, viewport_h);

    Rml::SetRenderInterface(g_renderer.get());
    Rml::SetSystemInterface(g_system.get());
    Rml::SetFileInterface(g_file.get());

    if (!Rml::Initialise()) {
        g_renderer.reset();
        g_system.reset();
        g_file.reset();
        return 0;
    }
    Rml::Lua::Initialise();
    register_lua_commands();
    return 1;
}

void rmlui_shutdown(void)
{
    if (!g_renderer) {
        return;
    }
    g_callbacks = nullptr;
    Rml::Shutdown();
    g_renderer.reset();
    g_system.reset();
    g_file.reset();
}

void rmlui_set_viewport(int w, int h)
{
    g_viewport_w = w;
    g_viewport_h = h;
    if (g_renderer) {
        if (auto* proxy = dynamic_cast<RustRenderProxy*>(g_renderer.get()))
            proxy->SetViewport(w, h);
    }
}

rmlui_handle_t rmlui_context_create(const char* name)
{
    if (!g_renderer) return nullptr;
    Rml::Context* ctx = Rml::CreateContext(name ? name : "default",
        Rml::Vector2i(g_viewport_w, g_viewport_h));
    return (rmlui_handle_t)ctx;
}

void rmlui_context_destroy(rmlui_handle_t ctx)
{
    if (ctx) {
        Rml::RemoveContext(((Rml::Context*)ctx)->GetName());
    }
}

void rmlui_template_cache_clear(void)
{
    Rml::Factory::ClearTemplateCache();
}

void rmlui_style_sheet_cache_clear(void)
{
    Rml::Factory::ClearStyleSheetCache();
}

int rmlui_context_remove_data_model(rmlui_handle_t ctx, const char* name)
{
    if (!ctx || !name || !name[0]) return 0;
    return ((Rml::Context*)ctx)->RemoveDataModel(name) ? 1 : 0;
}

void rmlui_lua_do_string(const char* code)
{
    if (!code || !code[0]) return;
    Rml::String s(code);
    Rml::Lua::Interpreter::DoString(s, s);
}

int rmlui_context_set_data_model_bool(rmlui_handle_t ctx, const char* model_name, const char* var_name, int value)
{
    // Newer RmlUi removed DataModelHandle::SetVariable; bind vars via constructor instead.
    (void)ctx; (void)model_name; (void)var_name; (void)value;
    return 0;
}

rmlui_handle_t rmlui_document_load(rmlui_handle_t ctx, const char* path)
{
    if (!ctx || !path) return nullptr;
    Rml::ElementDocument* doc = ((Rml::Context*)ctx)->LoadDocument(path);
    return (rmlui_handle_t)doc;
}

void rmlui_document_show(rmlui_handle_t doc)
{
    if (doc) {
        ((Rml::ElementDocument*)doc)->Show();
    }
}

void rmlui_document_hide(rmlui_handle_t doc)
{
    if (doc) {
        ((Rml::ElementDocument*)doc)->Hide();
    }
}

void rmlui_document_close(rmlui_handle_t doc)
{
    if (doc) {
        ((Rml::ElementDocument*)doc)->Close();
    }
}

void rmlui_document_dispatch_event(rmlui_handle_t doc, const char* event_type)
{
    if (!doc || !event_type || !event_type[0]) return;
    Rml::ElementDocument* d = (Rml::ElementDocument*)doc;
    Rml::Dictionary params;
    d->DispatchEvent(Rml::String(event_type), params, true, true);
}

void rmlui_context_update(rmlui_handle_t ctx)
{
    if (ctx) {
        ((Rml::Context*)ctx)->Update();
    }
}

void rmlui_context_render(rmlui_handle_t ctx)
{
    if (!ctx || !g_renderer) return;
    if (auto* proxy = dynamic_cast<RustRenderProxy*>(g_renderer.get()))
        proxy->BeginFrame();
    ((Rml::Context*)ctx)->Render();
    if (auto* proxy = dynamic_cast<RustRenderProxy*>(g_renderer.get()))
        proxy->EndFrame();
}

void rmlui_context_set_dimensions(rmlui_handle_t ctx, int w, int h)
{
    if (ctx && w > 0 && h > 0) {
        ((Rml::Context*)ctx)->SetDimensions(Rml::Vector2i(w, h));
    }
}

void rmlui_process_mouse_move(rmlui_handle_t ctx, int x, int y)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessMouseMove(x, y, 0);
    }
}

void rmlui_process_mouse_button_down(rmlui_handle_t ctx, int button)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessMouseButtonDown(button, 0);
    }
}

void rmlui_process_mouse_button_up(rmlui_handle_t ctx, int button)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessMouseButtonUp(button, 0);
    }
}

void rmlui_process_mouse_wheel(rmlui_handle_t ctx, float wheel_delta)
{
    if (ctx && wheel_delta != 0.0f) {
        ((Rml::Context*)ctx)->ProcessMouseWheel(wheel_delta, 0);
    }
}

void rmlui_process_key_down(rmlui_handle_t ctx, int key)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessKeyDown((Rml::Input::KeyIdentifier)key, 0);
    }
}

void rmlui_process_key_up(rmlui_handle_t ctx, int key)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessKeyUp((Rml::Input::KeyIdentifier)key, 0);
    }
}

void rmlui_process_text_input(rmlui_handle_t ctx, unsigned int codepoint)
{
    if (ctx) {
        ((Rml::Context*)ctx)->ProcessTextInput((Rml::Character)codepoint);
    }
}

int rmlui_load_font(const char* path)
{
    return Rml::LoadFontFace(path) ? 1 : 0;
}
