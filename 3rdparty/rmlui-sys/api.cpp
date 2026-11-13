#include "api.h"
#include "rust_bridge_proxy.h"

#include <RmlUi/Core/Core.h>
#include <RmlUi/Core/Context.h>
#include <RmlUi/Core/DataModelHandle.h>
#include <RmlUi/Core/ElementDocument.h>
#include <RmlUi/Core/ElementText.h>
#include <RmlUi/Core/Factory.h>
#include <RmlUi/Core/Input.h>
#include <RmlUi/Core/Log.h>
#include <RmlUi/Core/Traits.h>
#include <RmlUi/Core/Variant.h>
#include <RmlUi/Lua/Lua.h>
#include <RmlUi/Lua/Interpreter.h>
#include <RmlUi/Lua/IncludeLua.h>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>

static std::unique_ptr<Rml::RenderInterface> g_renderer;
static RustRenderProxy* g_proxy = nullptr;
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

static int lua_eng_time(lua_State* L)
{
    if (!g_callbacks || !g_callbacks->get_time) {
        lua_pushnumber(L, 0);
        return 1;
    }
    lua_pushnumber(L, g_callbacks->get_time());
    return 1;
}

// Latest context dp ratio (see rmlui_context_set_dp_ratio). The context space
// is framebuffer pixels while CSS `px`/`dp` lengths are scaled by this ratio,
// so UI scripts need it to convert event mouse coordinates (context pixels)
// to CSS lengths. Bit-cast through u32 keeps the static race-free without a
// mutex; writers are the single UI thread.
static std::atomic<uint32_t> g_dp_ratio_bits{0x3f800000}; // 1.0f

static int lua_eng_dp_ratio(lua_State* L)
{
    const uint32_t bits = g_dp_ratio_bits.load(std::memory_order_relaxed);
    float ratio = 1.0f;
    memcpy(&ratio, &bits, sizeof(ratio));
    lua_pushnumber(L, ratio > 0.f ? ratio : 1.0f);
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
    lua_pushcfunction(L, lua_eng_time);
    lua_setglobal(L, "eng_time");
    lua_pushcfunction(L, lua_eng_dp_ratio);
    lua_setglobal(L, "eng_dp_ratio");
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
    auto proxy = std::make_unique<RustRenderProxy>(render_api, callbacks);
    g_proxy = proxy.get();
    g_proxy->SetViewport(viewport_w, viewport_h);
    g_renderer = std::move(proxy);

    Rml::SetRenderInterface(g_renderer.get());
    Rml::SetSystemInterface(g_system.get());
    Rml::SetFileInterface(g_file.get());

    if (!Rml::Initialise()) {
        g_proxy = nullptr;
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
    g_proxy = nullptr;
    g_renderer.reset();
    g_system.reset();
    g_file.reset();
}

void rmlui_set_viewport(int w, int h)
{
    g_viewport_w = w;
    g_viewport_h = h;
    if (g_proxy)
        g_proxy->SetViewport(w, h);
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

static constexpr const char* kLuaDataModelsRegistryKey = "datamodels";
static constexpr const char* kLuaDataModelMetatable = "RMLDATAMODEL";
// Metatable name used by RmlUi's Lua TableProxy userdata (LuaDataModel.cpp).
static constexpr const char* kLuaTableProxyMetatable = "RMLTABLEPROXY";

// Push the named data-model userdata (registry `datamodels[name]`); nothing is
// pushed when the model is unknown or the interpreter is missing.
static bool rmlui_apply_push_model(lua_State* L, const char* model_name)
{
    if (!L || !model_name || !model_name[0])
        return false;

    lua_pushstring(L, kLuaDataModelsRegistryKey);
    lua_rawget(L, LUA_REGISTRYINDEX);
    if (!lua_istable(L, -1)) {
        lua_pop(L, 1);
        return false;
    }
    lua_pushstring(L, model_name);
    lua_rawget(L, -2);
    lua_remove(L, -2);
    if (!lua_isuserdata(L, -1) || !luaL_testudata(L, -1, kLuaDataModelMetatable)) {
        lua_pop(L, 1);
        return false;
    }
    return true;
}

static int rmlui_data_model_set_field(lua_State* L, const char* model_name, const char* field_name, int value_stack_index)
{
    if (!field_name || !field_name[0])
        return 0;

    const int value_abs = lua_absindex(L, value_stack_index);
    if (!rmlui_apply_push_model(L, model_name))
        return 0;

    lua_pushvalue(L, value_abs);
    lua_setfield(L, -2, field_name);
    lua_pop(L, 1);
    return 1;
}

static Rml::String attribute_string(const Rml::Element* el, const char* name)
{
    if (!el)
        return {};
    const Rml::Variant* v = el->GetAttribute(name);
    if (!v || v->GetType() != Rml::Variant::STRING)
        return {};
    return v->Get<Rml::String>();
}

static Rml::String rmlui_document_data_model_name(Rml::ElementDocument* doc)
{
    if (!doc)
        return {};
    Rml::String model = attribute_string(doc, "data-model");
    if (!model.empty())
        return model;
    for (int i = 0; i < doc->GetNumChildren(); ++i) {
        Rml::Element* child = doc->GetChild(i);
        if (!child)
            continue;
        Rml::String tag = child->GetTagName();
        if (tag == "body" || tag == "BODY") {
            model = attribute_string(child, "data-model");
            if (!model.empty())
                return model;
        }
    }
    return {};
}

int rmlui_data_model_set_bool(const char* model_name, const char* field_name, int value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushboolean(L, value);
    const int ok = rmlui_data_model_set_field(L, model_name, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

int rmlui_data_model_set_number(const char* model_name, const char* field_name, double value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushnumber(L, value);
    const int ok = rmlui_data_model_set_field(L, model_name, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

int rmlui_data_model_set_string(const char* model_name, const char* field_name, const char* value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushstring(L, value ? value : "");
    const int ok = rmlui_data_model_set_field(L, model_name, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

// ---- batched data-model writes -------------------------------------------
//
// Walks dotted paths through the RmlUi Lua data-model mechanics: reads go
// through __index (lDataModelGet / TableProxyGet), writes through __newindex
// (lDataModelSet / TableProxySet), so dirtying and the proxy value/table
// stores behave exactly like Lua-side writes. Nothing here may raise a Lua
// error (that would longjmp across the Rust FFI): every lua_gettable /
// lua_settable is guarded by an explicit indexability check.

namespace {

constexpr int kApplyMaxSegments = 32;
// Probe/resize guard: data models drive widgets, arrays stay in the thousands.
constexpr long long kApplyMaxResizeLen = 1 << 20;

struct ApplySegment {
    const char* begin;
    size_t len;
};

bool apply_segment_is_digits(const ApplySegment& seg)
{
    if (seg.len == 0)
        return false;
    for (size_t i = 0; i < seg.len; ++i) {
        if (seg.begin[i] < '0' || seg.begin[i] > '9')
            return false;
    }
    return true;
}

// Split "a.b.3" on '.'; false on empty segments or more than kApplyMaxSegments.
bool apply_split_path(const char* path, ApplySegment segs[kApplyMaxSegments], int* count)
{
    *count = 0;
    const char* p = path;
    for (;;) {
        const char* dot = strchr(p, '.');
        const size_t len = dot ? (size_t)(dot - p) : strlen(p);
        if (len == 0 || *count >= kApplyMaxSegments)
            return false;
        segs[*count].begin = p;
        segs[*count].len = len;
        ++*count;
        if (!dot)
            return true;
        p = dot + 1;
    }
}

// Digit-only segments are 1-based Lua array indices; anything else (including
// digit strings too long for an index) is a string key.
bool apply_push_key(lua_State* L, const ApplySegment& seg)
{
    char buf[24];
    if (apply_segment_is_digits(seg) && seg.len < sizeof(buf)) {
        memcpy(buf, seg.begin, seg.len);
        buf[seg.len] = '\0';
        char* end = nullptr;
        const long long index = strtoll(buf, &end, 10);
        if (end && *end == '\0' && index >= 1) {
            lua_pushinteger(L, (lua_Integer)index);
            return true;
        }
        return false;
    }
    lua_pushlstring(L, seg.begin, seg.len);
    return true;
}

// True for values the walk may index without Lua raising an error.
bool apply_is_indexable(lua_State* L, int index)
{
    if (lua_istable(L, index))
        return true;
    if (luaL_testudata(L, index, kLuaDataModelMetatable))
        return true;
    return luaL_testudata(L, index, kLuaTableProxyMetatable) != nullptr;
}

// Walk `count` segments starting from the value at `base`; on success the
// final value is on top of the stack (at base + count) and the intermediate
// containers sit at base..base+count-1. `base` itself is never removed.
bool apply_walk(lua_State* L, int base, const ApplySegment* segs, int count, const char* path, char* err, size_t err_size)
{
    for (int i = 0; i < count; ++i) {
        const int container = base + i;
        if (!apply_is_indexable(L, container)) {
            snprintf(err, err_size, "path '%s': segment %d is not a table", path, i);
            return false;
        }
        if (!apply_push_key(L, segs[i])) {
            snprintf(err, err_size, "path '%s': segment %d is not a valid array index", path, i);
            return false;
        }
        // Each result lands one slot above its container, so the walked chain
        // naturally sits at base+1..base+count with the final value on top.
        lua_gettable(L, container);
    }
    return true;
}

// Re-writes the root variable through the model userdata so lDataModelSet
// fires DirtyVariable. Covers writes TableProxySet stores without dirtying
// (keys not seen before on that table). The walk already proved the root is a
// non-nil string variable.
void apply_dirty_root(lua_State* L, int base, const ApplySegment& root)
{
    lua_pushlstring(L, root.begin, root.len);
    lua_gettable(L, base);
    if (lua_isnil(L, -1)) {
        lua_pop(L, 1);
        return;
    }
    lua_pushlstring(L, root.begin, root.len);
    lua_pushvalue(L, -2);
    lua_settable(L, base);
    lua_pop(L, 1);
}

// Push the op value (bool / number / string). RESIZE is handled separately.
bool apply_push_value(lua_State* L, const rmlui_data_model_op_t& op, char* err, size_t err_size)
{
    switch (op.tag) {
    case RMLUI_DATA_MODEL_OP_BOOL:
        lua_pushboolean(L, op.value.b != 0);
        return true;
    case RMLUI_DATA_MODEL_OP_NUMBER:
        lua_pushnumber(L, op.value.num);
        return true;
    case RMLUI_DATA_MODEL_OP_STRING:
        if (!op.value.str) {
            snprintf(err, err_size, "null string value");
            return false;
        }
        lua_pushstring(L, op.value.str);
        return true;
    default:
        snprintf(err, err_size, "tag %d carries no value", (int)op.tag);
        return false;
    }
}

// One set op: walk to the parent container, then write the last segment
// through __newindex. Returns 0 / -1 with `err` filled.
int apply_set_op(lua_State* L, int base, const rmlui_data_model_op_t& op, const ApplySegment* segs, int count, char* err, size_t err_size)
{
    if (!apply_walk(L, base, segs, count - 1, op.path, err, err_size))
        return -1;
    // Walked values sit at base+1..base+count-1 (the model stays at base); the
    // parent of the final segment is the last walked value. The walk only
    // guards intermediates, so the parent needs its own nil check.
    const int parent = count == 1 ? base : base + count - 1;
    if (count > 1 && !apply_is_indexable(L, parent)) {
        snprintf(err, err_size, "path '%s': segment %d is not a table", op.path, count - 2);
        return -1;
    }

    // A pre-read through __index caches the key in the proxy table store, so
    // the write below hits TableProxySet's dirty branch whenever the field
    // exists. Writes to brand-new keys stay undirtied upstream; the root
    // self-assign below reports those.
    if (!apply_push_key(L, segs[count - 1])) {
        snprintf(err, err_size, "path '%s': segment %d is not a valid array index", op.path, count - 1);
        return -1;
    }
    lua_gettable(L, parent);
    const bool existed = !lua_isnil(L, -1);
    lua_pop(L, 1);

    if (!apply_push_key(L, segs[count - 1])) {
        snprintf(err, err_size, "path '%s': segment %d is not a valid array index", op.path, count - 1);
        return -1;
    }
    if (!apply_push_value(L, op, err, err_size))
        return -1;
    lua_settable(L, parent);

    if (!existed)
        apply_dirty_root(L, base, segs[0]);
    return 0;
}

// One resize op: replace the array at `path` with a rebuilt Lua array of the
// requested length (old elements kept, growth filled with empty tables). A
// wholesale replacement is what makes the new length visible to data views —
// per-element proxy writes would leave '#' (binding Size) stale. The write
// goes through the parent's __newindex, whose key the walk just cached, so
// TableProxySet / lDataModelSet fire DirtyVariable.
int apply_resize_op(lua_State* L, int base, const rmlui_data_model_op_t& op, const ApplySegment* segs, int count, char* err, size_t err_size)
{
    if (op.value.len < 0) {
        snprintf(err, err_size, "resize '%s': negative length", op.path);
        return -1;
    }
    if (op.value.len > kApplyMaxResizeLen) {
        snprintf(err, err_size, "resize '%s': length beyond %lld", op.path, kApplyMaxResizeLen);
        return -1;
    }
    if (!apply_walk(L, base, segs, count, op.path, err, err_size))
        return -1;
    const int container = base + count;
    // The array's parent container is the second-to-last walked value.
    const int parent = count == 1 ? base : base + count - 1;

    const bool proxy = luaL_testudata(L, container, kLuaTableProxyMetatable) != nullptr;
    if (!proxy && !lua_istable(L, container)) {
        snprintf(err, err_size, "resize '%s': path does not reference a table", op.path);
        return -1;
    }

    // Current length: probe through __index. TableProxyGet falls back to the
    // underlying table, so this matches what data views see.
    long long current = 0;
    while (current < kApplyMaxResizeLen) {
        lua_pushinteger(L, (lua_Integer)(current + 1));
        lua_gettable(L, container);
        const bool nil = lua_isnil(L, -1) != 0;
        lua_pop(L, 1);
        if (nil)
            break;
        ++current;
    }
    if (current >= kApplyMaxResizeLen) {
        snprintf(err, err_size, "resize '%s': array longer than %lld", op.path, kApplyMaxResizeLen);
        return -1;
    }
    if (op.value.len == current) {
        lua_pop(L, 1);
        return 0; // already the requested length
    }

    const long long keep = current < op.value.len ? current : op.value.len;
    lua_createtable(L, (int)op.value.len, 0);
    const int fresh = lua_gettop(L);
    for (long long i = 1; i <= keep; ++i) {
        lua_pushinteger(L, (lua_Integer)i);
        lua_gettable(L, container);
        lua_rawseti(L, fresh, (int)i);
    }
    for (long long i = keep + 1; i <= op.value.len; ++i) {
        lua_createtable(L, 0, 0);
        lua_rawseti(L, fresh, (int)i);
    }

    if (!apply_push_key(L, segs[count - 1])) {
        snprintf(err, err_size, "path '%s': segment %d is not a valid array index", op.path, count - 1);
        return -1;
    }
    lua_pushvalue(L, fresh);
    lua_settable(L, parent);
    lua_pop(L, 1); // the rebuilt array
    return 0;
}

int apply_one_op(lua_State* L, int base, const rmlui_data_model_op_t& op, char* err, size_t err_size)
{
    err[0] = '\0';
    if (!op.path || !op.path[0]) {
        snprintf(err, err_size, "empty path");
        return -1;
    }
    ApplySegment segs[kApplyMaxSegments];
    int count = 0;
    if (!apply_split_path(op.path, segs, &count)) {
        snprintf(err, err_size, "path '%s': empty segment or more than %d segments", op.path, kApplyMaxSegments);
        return -1;
    }
    // Dirtying and data binding are string-keyed on the root variable.
    if (apply_segment_is_digits(segs[0])) {
        snprintf(err, err_size, "path '%s': root segment must be a variable name", op.path);
        return -1;
    }

    switch (op.tag) {
    case RMLUI_DATA_MODEL_OP_BOOL:
    case RMLUI_DATA_MODEL_OP_NUMBER:
    case RMLUI_DATA_MODEL_OP_STRING:
        return apply_set_op(L, base, op, segs, count, err, err_size);
    case RMLUI_DATA_MODEL_OP_RESIZE:
        return apply_resize_op(L, base, op, segs, count, err, err_size);
    default:
        snprintf(err, err_size, "unknown op tag %d", (int)op.tag);
        return -1;
    }
}

int rmlui_data_model_apply_ops(lua_State* L, const char* model_name, const rmlui_data_model_op_t* ops, size_t ops_len)
{
    if (!L) {
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: Lua interpreter not running");
        return -1;
    }
    if (!ops && ops_len > 0) {
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: ops is null with %zu ops", ops_len);
        return -1;
    }
    if (!rmlui_apply_push_model(L, model_name)) {
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: unknown data model '%s'",
            model_name ? model_name : "(null)");
        return -1;
    }

    const int base = lua_gettop(L);
    if (!lua_checkstack(L, 2 * kApplyMaxSegments + 16)) {
        lua_pop(L, 1);
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: out of Lua stack");
        return -1;
    }

    for (size_t i = 0; i < ops_len; ++i) {
        char err[256];
        if (apply_one_op(L, base, ops[i], err, sizeof(err)) != 0) {
            Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: model '%s' op %zu: %s",
                model_name, i, err);
            lua_settop(L, base - 1);
            return -(int)(i + 1);
        }
        // Drop the op's walk chain; the model stays anchored at `base`.
        lua_settop(L, base);
    }
    lua_settop(L, base - 1);
    return 0;
}

} // namespace

int rmlui_data_model_apply(const char* model_name, const rmlui_data_model_op_t* ops, size_t ops_len)
{
    return rmlui_data_model_apply_ops(Rml::Lua::Interpreter::GetLuaState(), model_name, ops, ops_len);
}

int rmlui_document_data_model_apply(rmlui_handle_t doc, const rmlui_data_model_op_t* ops, size_t ops_len)
{
    if (!doc) {
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: null document");
        return -1;
    }
    const Rml::String model = rmlui_document_data_model_name((Rml::ElementDocument*)doc);
    if (model.empty()) {
        Rml::Log::Message(Rml::Log::LT_ERROR, "data_model_apply: document has no data-model");
        return -1;
    }
    return rmlui_data_model_apply_ops(Rml::Lua::Interpreter::GetLuaState(), model.c_str(), ops, ops_len);
}

static int rmlui_document_data_model_set_field(rmlui_handle_t doc, const char* field_name, int value_stack_index)
{
    if (!doc)
        return 0;
    const Rml::String model = rmlui_document_data_model_name((Rml::ElementDocument*)doc);
    if (model.empty())
        return 0;
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    return rmlui_data_model_set_field(L, model.c_str(), field_name, value_stack_index);
}

int rmlui_document_data_model_set_bool(rmlui_handle_t doc, const char* field_name, int value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushboolean(L, value);
    const int ok = rmlui_document_data_model_set_field(doc, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

int rmlui_document_data_model_set_number(rmlui_handle_t doc, const char* field_name, double value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushnumber(L, value);
    const int ok = rmlui_document_data_model_set_field(doc, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

int rmlui_document_data_model_set_string(rmlui_handle_t doc, const char* field_name, const char* value)
{
    lua_State* L = Rml::Lua::Interpreter::GetLuaState();
    if (!L)
        return 0;
    lua_pushstring(L, value ? value : "");
    const int ok = rmlui_document_data_model_set_field(doc, field_name, -1);
    lua_pop(L, 1);
    return ok;
}

int rmlui_context_set_data_model_bool(rmlui_handle_t ctx, const char* model_name, const char* var_name, int value)
{
    (void)ctx;
    return rmlui_data_model_set_bool(model_name, var_name, value);
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

int rmlui_context_update(rmlui_handle_t ctx)
{
    if (!ctx) {
        return 0;
    }
    ((Rml::Context*)ctx)->Update();
    return 1;
}

double rmlui_context_next_update_delay(rmlui_handle_t ctx)
{
    if (!ctx) {
        return 0.0;
    }
    return ((Rml::Context*)ctx)->GetNextUpdateDelay();
}

void rmlui_context_render(rmlui_handle_t ctx)
{
    if (!ctx || !g_proxy) return;
    g_proxy->BeginFrame();
    ((Rml::Context*)ctx)->Render();
    g_proxy->EndFrame();
}

void rmlui_context_set_dimensions(rmlui_handle_t ctx, int w, int h)
{
    if (ctx && w > 0 && h > 0) {
        ((Rml::Context*)ctx)->SetDimensions(Rml::Vector2i(w, h));
    }
}

void rmlui_context_set_dp_ratio(rmlui_handle_t ctx, float ratio)
{
    if (ctx && ratio > 0.f) {
        uint32_t bits;
        memcpy(&bits, &ratio, sizeof(bits));
        g_dp_ratio_bits.store(bits, std::memory_order_relaxed);
        ((Rml::Context*)ctx)->SetDensityIndependentPixelRatio(ratio);
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

// ---- document tree dump (read-only; agent tooling) -----------------------

#include <RmlUi/Core/ElementText.h>
#include <RmlUi/Core/Log.h>
#include <RmlUi/Core/Traits.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>

static void rmlui_tree_append_classes(Rml::String& out, const Rml::String& classes)
{
    // `GetClassNames()` yields a space-separated list; emit `.a.b`.
    size_t begin = 0;
    while (begin < classes.size()) {
        const size_t end = classes.find(' ', begin);
        const size_t stop = (end == Rml::String::npos) ? classes.size() : end;
        if (stop > begin) {
            out.push_back('.');
            out.append(classes, begin, stop - begin);
        }
        begin = stop + 1;
    }
}

static void rmlui_tree_append_text(Rml::String& out, const Rml::String& raw)
{
    // Collapse whitespace runs to single spaces, drop leading/trailing.
    Rml::String collapsed;
    bool pending_space = false;
    for (const char c : raw) {
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            pending_space = !collapsed.empty();
            continue;
        }
        if (pending_space) {
            collapsed.push_back(' ');
            pending_space = false;
        }
        collapsed.push_back(c);
    }
    if (collapsed.empty())
        return;
    static constexpr size_t kMaxText = 60;
    if (collapsed.size() > kMaxText) {
        size_t keep = kMaxText;
        while (keep > 0 && (collapsed[keep] & 0xC0) == 0x80) // no UTF-8 code point splits
            --keep;
        collapsed.resize(keep);
        collapsed.append("...");
    }
    out.append(" \"");
    out.append(collapsed);
    out.push_back('"');
}

static void rmlui_tree_walk(Rml::Element* el, int depth, int max_depth, Rml::String& out)
{
    out.append(static_cast<size_t>(depth) * 2, ' ');
    out.append(el->GetTagName());
    const Rml::String& id = el->GetId();
    if (!id.empty()) {
        out.push_back('#');
        out.append(id);
    }
    rmlui_tree_append_classes(out, el->GetClassNames());

    // Post-layout border box: position relative to the document (context px), size.
    const Rml::Vector2f pos = el->GetAbsoluteOffset(Rml::BoxArea::Border);
    const Rml::Vector2f size = el->GetBox().GetSize(Rml::BoxArea::Border);
    char box[96];
    std::snprintf(box, sizeof(box), " [%.0f,%.0f %.0fx%.0f]", pos.x, pos.y, size.x, size.y);
    out.append(box);

    if (const Rml::ElementText* text = rmlui_dynamic_cast<const Rml::ElementText*>(el))
        rmlui_tree_append_text(out, text->GetText());
    out.push_back('\n');

    // max_depth = maximum nesting levels printed (1 = root only); <= 0 = unlimited.
    if (max_depth > 0 && depth + 1 >= max_depth)
        return;
    const int num_children = el->GetNumChildren(false);
    for (int i = 0; i < num_children; ++i)
        rmlui_tree_walk(el->GetChild(i), depth + 1, max_depth, out);
}

char* rmlui_document_tree(rmlui_handle_t doc, int max_depth)
{
    auto* root = static_cast<Rml::ElementDocument*>(doc);
    if (!root)
        return nullptr;
    Rml::String out;
    rmlui_tree_walk(root, 0, max_depth, out);
    char* buffer = static_cast<char*>(std::malloc(out.size() + 1));
    if (!buffer)
        return nullptr;
    std::memcpy(buffer, out.c_str(), out.size() + 1);
    return buffer;
}

void rmlui_free(void* ptr)
{
    std::free(ptr);
}
