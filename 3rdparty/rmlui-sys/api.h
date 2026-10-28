/**
 * rmlui-sys — C API для RmlUi.
 * Библиотека не знает про Xash. Caller передаёт render_api (функции рендера).
 * Связь с triapi/ref делается в Rust (crate menu).
 */
#ifndef RMLUI_SYS_API_H
#define RMLUI_SYS_API_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>

typedef void* rmlui_handle_t;

/** Callbacks от caller (время, лог, base_path, scissor, execute_command, client_in_game, get_cvar_float) */
typedef struct rmlui_callbacks_s {
    double (*get_time)(void);
    void (*log)(int level, const char* msg);
    const char* (*get_base_path)(void);
    void (*scissor)(int x, int y, int w, int h);
    /** Выполнить консольную команду (опционально). Для Lua: cmd("disconnect") и т.п. */
    void (*execute_command)(const char* cmd);
    /** 1 = в игре (ClientInGame), 0 = в меню. Для Lua: in_game() */
    int (*client_in_game)(void);
    /** Значение cvar по имени. Для Lua: cvar_float("cl_background") */
    float (*get_cvar_float)(const char* name);
    /** Строковое значение cvar. Для Lua: cvar_string("name") */
    const char* (*get_cvar_string)(const char* name);
    /** Список файлов по маске (gamedir). Возвращает JSON-массив путей. Для Lua: eng_get_files_list("matches/*.json") */
    const char* (*get_files_list)(const char* pattern);
    /** Загрузить файл из gamedir. Возвращает содержимое или NULL при ошибке. Для Lua: eng_load_file("matches/123.json") */
    const char* (*load_file)(const char* path);
} rmlui_callbacks_t;

/** Render API — caller передаёт функции рендера (triapi/ref или др.) */
typedef struct rmlui_render_api_s {
    void (*R_Set2DMode)(int enable);
    void (*TriRenderMode)(int mode);
    void (*Begin)(int mode);
    void (*End)(void);
    void (*Color4ub)(unsigned char r, unsigned char g, unsigned char b, unsigned char a);
    void (*TexCoord2f)(float u, float v);
    void (*Vertex3f)(float x, float y, float z);
    void (*GL_Bind)(int tmu, unsigned int texnum);
    void (*GL_SetRenderMode)(int mode);
    int (*GL_LoadTexture)(const char* name, const unsigned char* buf, size_t size, int flags);
    int (*GL_CreateTexture)(const char* name, int width, int height, const void* buf, unsigned int flags);
    void (*FillRGBA)(int rendermode, float x, float y, float w, float h, unsigned char r, unsigned char g, unsigned char b, unsigned char a);
    void (*R_DrawStretchPic)(float x, float y, float w, float h, float s1, float t1, float s2, float t2, int texnum);
    /** Pre-loaded *white texture for background-color-only geometry. 0 = not set. */
    unsigned int white_texture;
} rmlui_render_api_t;

#define RMLUI_LOG_ERROR 0
#define RMLUI_LOG_WARN  1
#define RMLUI_LOG_INFO  2

#define RMLUI_RENDER_TRANS_TEXTURE 2
#define RMLUI_TRI_TRIANGLES 0

/**
 * Инициализация. render_api обязателен — без него init не работает.
 * @return 1 на успех, 0 на ошибку
 */
int rmlui_init(int viewport_w, int viewport_h, const rmlui_callbacks_t* callbacks, const rmlui_render_api_t* render_api);

void rmlui_shutdown(void);
void rmlui_set_viewport(int w, int h);

rmlui_handle_t rmlui_context_create(const char* name);
void rmlui_context_destroy(rmlui_handle_t ctx);

/** Очистить кэш шаблонов — при следующей загрузке документов шаблоны будут перечитаны с диска. */
void rmlui_template_cache_clear(void);
/** Очистить кэш стилей (RCSS) — при следующей загрузке документов стили будут перечитаны с диска. */
void rmlui_style_sheet_cache_clear(void);

/** Удалить data model из контекста. Нужно перед menu_reload, иначе "attach a new data model or detach the old one". */
int rmlui_context_remove_data_model(rmlui_handle_t ctx, const char* name);
/** Выполнить Lua-код (для сброса MenuState.datamodel, Options.datamodel перед reload). */
void rmlui_lua_do_string(const char* code);

/** @deprecated use rmlui_data_model_set_bool */
int rmlui_context_set_data_model_bool(rmlui_handle_t ctx, const char* model_name, const char* var_name, int value);

/** Set a field on a named Lua data-model (opened via OpenDataModel). */
int rmlui_data_model_set_bool(const char* model_name, const char* field_name, int value);
int rmlui_data_model_set_number(const char* model_name, const char* field_name, double value);
int rmlui_data_model_set_string(const char* model_name, const char* field_name, const char* value);

/**
 * Set a field on the document's body `data-model` (widget / document handle).
 * @return 1 on success, 0 if document/model/field missing.
 */
int rmlui_document_data_model_set_bool(rmlui_handle_t doc, const char* field_name, int value);
int rmlui_document_data_model_set_number(rmlui_handle_t doc, const char* field_name, double value);
int rmlui_document_data_model_set_string(rmlui_handle_t doc, const char* field_name, const char* value);

rmlui_handle_t rmlui_document_load(rmlui_handle_t ctx, const char* path);
void rmlui_document_show(rmlui_handle_t doc);
void rmlui_document_hide(rmlui_handle_t doc);
void rmlui_document_close(rmlui_handle_t doc);
/** Отправить кастомное событие в документ (bubbles). Для RML: data-event-stateupdated="..." или onstateupdated="..." */
void rmlui_document_dispatch_event(rmlui_handle_t doc, const char* event_type);

void rmlui_context_update(rmlui_handle_t ctx);
void rmlui_context_render(rmlui_handle_t ctx);
void rmlui_context_set_dimensions(rmlui_handle_t ctx, int w, int h);

void rmlui_process_mouse_move(rmlui_handle_t ctx, int x, int y);
void rmlui_process_mouse_button_down(rmlui_handle_t ctx, int button);
void rmlui_process_mouse_button_up(rmlui_handle_t ctx, int button);
/** Positive delta = scroll up / away from user (RmlUi convention). */
void rmlui_process_mouse_wheel(rmlui_handle_t ctx, float wheel_delta);
void rmlui_process_key_down(rmlui_handle_t ctx, int key);
void rmlui_process_key_up(rmlui_handle_t ctx, int key);
void rmlui_process_text_input(rmlui_handle_t ctx, unsigned int codepoint);

int rmlui_load_font(const char* path);

#ifdef __cplusplus
}
#endif

#endif
