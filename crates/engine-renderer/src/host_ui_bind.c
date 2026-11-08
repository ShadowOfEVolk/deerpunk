#define NK_IMPLEMENTATION
#define NK_INCLUDE_DEFAULT_ALLOCATOR
#define NK_INCLUDE_STANDARD_IO
#define NK_INCLUDE_STANDARD_VARARGS
#define NK_INCLUDE_VERTEX_BUFFER_OUTPUT
#define NK_INCLUDE_FONT_BAKING
#include "nuklear.h"
#include "host_ui_proggyclean.inc"

#include <stdlib.h>
#include <string.h>

struct HostUi {
    struct nk_context ctx;
    struct nk_font_atlas atlas;
    struct nk_font *font;
    struct nk_draw_null_texture null_tex;
    struct nk_buffer cmds;
    struct nk_buffer vbuf;
    struct nk_buffer ibuf;
    struct nk_convert_config config;
    int atlas_w;
    int atlas_h;
    unsigned char *atlas_pixels;
    int input_open;
    int overlay_style;
    struct nk_vec2 saved_spacing;
    struct nk_vec2 saved_padding;
};

struct HostUiDraw {
    const void *verts;
    int vert_stride;
    int vert_count;
    const void *indices;
    int index_count;
};

struct HostUiCmd {
    float x, y, w, h;
    int elem_count;
    int index_offset;
};

struct HostUi *host_ui_create(void) {
    struct HostUi *ui = (struct HostUi *)calloc(1, sizeof(struct HostUi));
    if (!ui) return 0;
    nk_font_atlas_init_default(&ui->atlas);
    nk_font_atlas_begin(&ui->atlas);
    /* ProggyClean 13px (TTF embedded via build.rs). */
    struct nk_font_config font_cfg = nk_font_config(13.0f);
    font_cfg.oversample_h = 1;
    font_cfg.oversample_v = 1;
    font_cfg.pixel_snap = nk_true;
    font_cfg.spacing = nk_vec2(0, 0);
    ui->font = nk_font_atlas_add_from_memory(
        &ui->atlas,
        (void *)host_ui_proggy_clean_ttf,
        (nk_size)host_ui_proggy_clean_ttf_size,
        13.0f,
        &font_cfg);
    int w = 0, h = 0;
    const void *image = nk_font_atlas_bake(&ui->atlas, &w, &h, NK_FONT_ATLAS_RGBA32);
    ui->atlas_w = w;
    ui->atlas_h = h;
    size_t bytes = (size_t)w * (size_t)h * 4;
    if (image && bytes) {
        ui->atlas_pixels = (unsigned char *)malloc(bytes);
        if (ui->atlas_pixels) {
            memcpy(ui->atlas_pixels, image, bytes);
            for (size_t i = 3; i < bytes; i += 4) {
                ui->atlas_pixels[i] = ui->atlas_pixels[i] >= 128 ? 255 : 0;
            }
        }
    }
    nk_init_default(&ui->ctx, &ui->font->handle);
    nk_buffer_init_default(&ui->cmds);
    nk_buffer_init_default(&ui->vbuf);
    nk_buffer_init_default(&ui->ibuf);
    return ui;
}

void host_ui_atlas_end(struct HostUi *ui) {
    if (!ui) return;
    nk_font_atlas_end(&ui->atlas, nk_handle_id(1), &ui->null_tex);
    nk_style_set_font(&ui->ctx, &ui->font->handle);
    memset(&ui->config, 0, sizeof(ui->config));
    static const struct nk_draw_vertex_layout_element layout[] = {
        {NK_VERTEX_POSITION, NK_FORMAT_FLOAT, 0},
        {NK_VERTEX_TEXCOORD, NK_FORMAT_FLOAT, 8},
        {NK_VERTEX_COLOR, NK_FORMAT_R8G8B8A8, 16},
        {NK_VERTEX_LAYOUT_END}
    };
    ui->config.vertex_layout = layout;
    ui->config.vertex_size = 20;
    ui->config.vertex_alignment = 4;
    ui->config.tex_null = ui->null_tex;
    ui->config.circle_segment_count = 22;
    ui->config.curve_segment_count = 22;
    ui->config.arc_segment_count = 22;
    ui->config.global_alpha = 1.0f;
    ui->config.shape_AA = NK_ANTI_ALIASING_OFF;
    ui->config.line_AA = NK_ANTI_ALIASING_OFF;
}

const unsigned char *host_ui_atlas_pixels(struct HostUi *ui, int *w, int *h) {
    if (!ui) return 0;
    if (w) *w = ui->atlas_w;
    if (h) *h = ui->atlas_h;
    return ui->atlas_pixels;
}

void host_ui_destroy(struct HostUi *ui) {
    if (!ui) return;
    nk_buffer_free(&ui->cmds);
    nk_buffer_free(&ui->vbuf);
    nk_buffer_free(&ui->ibuf);
    nk_font_atlas_clear(&ui->atlas);
    nk_free(&ui->ctx);
    free(ui->atlas_pixels);
    free(ui);
}

void host_ui_input_begin(struct HostUi *ui) {
    if (!ui) return;
    nk_input_begin(&ui->ctx);
    ui->input_open = 1;
}
void host_ui_motion(struct HostUi *ui, float x, float y) { if (ui) nk_input_motion(&ui->ctx, (int)x, (int)y); }
void host_ui_button(struct HostUi *ui, int button, float x, float y, int down) {
    if (ui) nk_input_button(&ui->ctx, (enum nk_buttons)button, (int)x, (int)y, down);
}
void host_ui_scroll(struct HostUi *ui, float y) { if (ui) nk_input_scroll(&ui->ctx, nk_vec2(0, y)); }
void host_ui_key(struct HostUi *ui, int key, int down) {
    if (ui) nk_input_key(&ui->ctx, (enum nk_keys)key, down);
}
void host_ui_unicode(struct HostUi *ui, unsigned code) { if (ui) nk_input_unicode(&ui->ctx, code); }
void host_ui_input_end(struct HostUi *ui) {
    if (!ui) return;
    nk_input_end(&ui->ctx);
    ui->input_open = 0;
}

int host_ui_key_down(struct HostUi *ui, int key) {
    if (!ui) return 0;
    return nk_input_is_key_down(&ui->ctx.input, (enum nk_keys)key);
}
int host_ui_key_pressed(struct HostUi *ui, int key) {
    if (!ui) return 0;
    return nk_input_is_key_pressed(&ui->ctx.input, (enum nk_keys)key);
}

int host_ui_begin(struct HostUi *ui, const char *title, float x, float y, float w, float h, int flags) {
    if (!ui || !title) return 0;
    /* Infobox: no input + background. Console shares the style, so restore after begin. */
    int overlay = ((flags & NK_WINDOW_NO_INPUT) != 0) && ((flags & NK_WINDOW_BACKGROUND) != 0);
    struct nk_style_item saved_bg;
    struct nk_color saved_col;
    struct nk_color saved_border;
    if (overlay) {
        struct nk_color c = nk_rgba(0, 0, 0, 150);
        saved_bg = ui->ctx.style.window.fixed_background;
        saved_col = ui->ctx.style.window.background;
        saved_border = ui->ctx.style.window.border_color;
        ui->ctx.style.window.fixed_background = nk_style_item_color(c);
        ui->ctx.style.window.background = c;
        ui->ctx.style.window.border_color = nk_rgba(255, 255, 255, 36);
        ui->saved_spacing = ui->ctx.style.window.spacing;
        ui->saved_padding = ui->ctx.style.window.padding;
        ui->ctx.style.window.spacing = nk_vec2(2.0f, 0.0f);
        ui->ctx.style.window.padding = nk_vec2(6.0f, 4.0f);
        ui->overlay_style = 1;
    }
    int open = nk_begin(&ui->ctx, title, nk_rect(x, y, w, h), (nk_flags)flags);
    if (overlay) {
        ui->ctx.style.window.fixed_background = saved_bg;
        ui->ctx.style.window.background = saved_col;
        ui->ctx.style.window.border_color = saved_border;
        if (!open) {
            ui->ctx.style.window.spacing = ui->saved_spacing;
            ui->ctx.style.window.padding = ui->saved_padding;
            ui->overlay_style = 0;
        }
    }
    return open;
}
void host_ui_end(struct HostUi *ui) {
    if (!ui) return;
    nk_end(&ui->ctx);
    if (ui->overlay_style) {
        ui->ctx.style.window.spacing = ui->saved_spacing;
        ui->ctx.style.window.padding = ui->saved_padding;
        ui->overlay_style = 0;
    }
}

int host_ui_flags_overlay(void) {
    return NK_WINDOW_NO_SCROLLBAR | NK_WINDOW_BORDER | NK_WINDOW_NO_INPUT | NK_WINDOW_BACKGROUND;
}
int host_ui_flags_panel(void) {
    return NK_WINDOW_BORDER | NK_WINDOW_NO_SCROLLBAR | NK_WINDOW_MOVABLE | NK_WINDOW_TITLE;
}
int host_ui_flags_console(void) {
    return NK_WINDOW_NO_SCROLLBAR | NK_WINDOW_BORDER | NK_WINDOW_BACKGROUND;
}

void host_ui_layout_dynamic(struct HostUi *ui, float h, int cols) {
    if (ui) nk_layout_row_dynamic(&ui->ctx, h, cols);
}
void host_ui_label(struct HostUi *ui, const char *text, float r, float g, float b, float a) {
    if (!ui || !text) return;
    nk_label_colored(&ui->ctx, text, NK_TEXT_LEFT, nk_rgba_f(r, g, b, a));
}
void host_ui_separator(struct HostUi *ui) {
    if (!ui) return;
    nk_layout_row_dynamic(&ui->ctx, 7.0f, 1);
    struct nk_rect b;
    if (!nk_widget(&b, &ui->ctx)) return;
    struct nk_command_buffer *canvas = nk_window_get_canvas(&ui->ctx);
    float y = (float)(int)(b.y + b.h * 0.5f) + 0.5f;
    nk_stroke_line(canvas, b.x, y, b.x + b.w, y, 1.0f, nk_rgba(255, 255, 255, 90));
}

int host_ui_group_begin(struct HostUi *ui, const char *id, float h) {
    if (!ui) return 0;
    nk_layout_row_dynamic(&ui->ctx, h, 1);
    return nk_group_begin(&ui->ctx, id, NK_WINDOW_BORDER);
}
void host_ui_group_end(struct HostUi *ui, const char *id, int follow_tail) {
    if (!ui) return;
    if (follow_tail) {
        nk_uint sx = 0, sy = 0;
        nk_group_get_scroll(&ui->ctx, id, &sx, &sy);
        nk_group_set_scroll(&ui->ctx, id, sx, 1000000);
    }
    nk_group_end(&ui->ctx);
}

int host_ui_selectable(struct HostUi *ui, const char *label, int selected) {
    if (!ui || !label) return 0;
    nk_bool sel = selected ? nk_true : nk_false;
    int changed = nk_selectable_label(&ui->ctx, label, NK_TEXT_LEFT, &sel);
    return changed || (sel && !selected);
}

void host_ui_edit_focus(struct HostUi *ui) { if (ui) nk_edit_focus(&ui->ctx, NK_EDIT_ALWAYS_INSERT_MODE); }

int host_ui_edit(struct HostUi *ui, char *buf, int cap) {
    if (!ui || !buf || cap <= 1) return 0;
    nk_flags flags = NK_EDIT_FIELD | NK_EDIT_SIG_ENTER | NK_EDIT_SELECTABLE | NK_EDIT_CLIPBOARD | NK_EDIT_GOTO_END_ON_ACTIVATE;
    return (int)nk_edit_string_zero_terminated(&ui->ctx, flags, buf, cap, nk_filter_default);
}

int host_ui_edit_committed(void) { return NK_EDIT_COMMITED; }

void host_ui_bar(struct HostUi *ui, int n, const float *x0, const float *x1, const unsigned char *rgba) {
    if (!ui || n < 0) return;
    struct nk_rect b;
    if (nk_widget(&b, &ui->ctx) == NK_WIDGET_INVALID) return;
    struct nk_command_buffer *canvas = nk_window_get_canvas(&ui->ctx);
    nk_fill_rect(canvas, b, 0, nk_rgb(28, 28, 32));
    for (int i = 0; i < n; i++) {
        float a = x0[i];
        float c = x1[i];
        if (c < a) continue;
        struct nk_rect s = b;
        s.x = b.x + a * b.w;
        s.w = (c - a) * b.w;
        if (s.w < 0.5f) continue;
        const unsigned char *col = rgba + i * 4;
        nk_fill_rect(canvas, s, 0, nk_rgba(col[0], col[1], col[2], col[3]));
    }
    nk_stroke_rect(canvas, b, 0, 1.0f, nk_rgb(140, 140, 150));
}

int host_ui_convert(struct HostUi *ui, struct HostUiCmd *cmds, int max_cmds, struct HostUiDraw *out) {
    if (!ui || !out) return 0;
    nk_buffer_clear(&ui->cmds);
    nk_buffer_clear(&ui->vbuf);
    nk_buffer_clear(&ui->ibuf);
    nk_convert(&ui->ctx, &ui->cmds, &ui->vbuf, &ui->ibuf, &ui->config);
    out->verts = nk_buffer_memory_const(&ui->vbuf);
    out->vert_stride = 20;
    out->vert_count = (int)(nk_buffer_total(&ui->vbuf) / 20);
    out->indices = nk_buffer_memory_const(&ui->ibuf);
    out->index_count = (int)(nk_buffer_total(&ui->ibuf) / sizeof(nk_draw_index));
    int n = 0;
    int offset = 0;
    const struct nk_draw_command *cmd;
    nk_draw_foreach(cmd, &ui->ctx, &ui->cmds) {
        if (!cmd->elem_count) continue;
        if (n >= max_cmds) break;
        cmds[n].x = cmd->clip_rect.x;
        cmds[n].y = cmd->clip_rect.y;
        cmds[n].w = cmd->clip_rect.w;
        cmds[n].h = cmd->clip_rect.h;
        cmds[n].elem_count = (int)cmd->elem_count;
        cmds[n].index_offset = offset;
        offset += (int)cmd->elem_count;
        n++;
    }
    return n;
}

int host_ui_index_size(void) { return (int)sizeof(nk_draw_index); }

void host_ui_finish(struct HostUi *ui) {
    if (!ui) return;
    nk_clear(&ui->ctx);
}

int host_ui_key_shift(void) { return NK_KEY_SHIFT; }
int host_ui_key_ctrl(void) { return NK_KEY_CTRL; }
int host_ui_key_enter(void) { return NK_KEY_ENTER; }
int host_ui_key_tab(void) { return NK_KEY_TAB; }
int host_ui_key_backspace(void) { return NK_KEY_BACKSPACE; }
int host_ui_key_up(void) { return NK_KEY_UP; }
int host_ui_key_down_key(void) { return NK_KEY_DOWN; }
int host_ui_key_left(void) { return NK_KEY_LEFT; }
int host_ui_key_right(void) { return NK_KEY_RIGHT; }
int host_ui_key_del(void) { return NK_KEY_DEL; }
