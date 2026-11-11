//! Host debug UI (console, infobox, alloc bars). Nuklear context stays in this process.
//! Widget calls are only valid between [`HostUi::begin_frame`] and [`HostUi::end_frame`].

use std::cell::Cell;
use std::ffi::CString;

#[repr(C)]
struct HostUiOpaque {
    _priv: [u8; 0],
}

#[repr(C)]
struct HostUiDraw {
    verts: *const u8,
    vert_stride: i32,
    vert_count: i32,
    indices: *const u8,
    index_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HostUiCmd {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    elem_count: i32,
    index_offset: i32,
}

unsafe extern "C" {
    fn host_ui_create() -> *mut HostUiOpaque;
    fn host_ui_atlas_end(ui: *mut HostUiOpaque);
    fn host_ui_atlas_pixels(ui: *mut HostUiOpaque, w: *mut i32, h: *mut i32) -> *const u8;
    fn host_ui_destroy(ui: *mut HostUiOpaque);
    fn host_ui_input_begin(ui: *mut HostUiOpaque);
    fn host_ui_motion(ui: *mut HostUiOpaque, x: f32, y: f32);
    fn host_ui_button(ui: *mut HostUiOpaque, button: i32, x: f32, y: f32, down: i32);
    fn host_ui_scroll(ui: *mut HostUiOpaque, y: f32);
    fn host_ui_key(ui: *mut HostUiOpaque, key: i32, down: i32);
    fn host_ui_unicode(ui: *mut HostUiOpaque, code: u32);
    fn host_ui_input_end(ui: *mut HostUiOpaque);
    fn host_ui_key_pressed(ui: *mut HostUiOpaque, key: i32) -> i32;
    fn host_ui_begin(
        ui: *mut HostUiOpaque,
        title: *const i8,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        flags: i32,
    ) -> i32;
    fn host_ui_end(ui: *mut HostUiOpaque);
    fn host_ui_flags_overlay() -> i32;
    fn host_ui_flags_panel() -> i32;
    fn host_ui_flags_console() -> i32;
    fn host_ui_layout_dynamic(ui: *mut HostUiOpaque, h: f32, cols: i32);
    fn host_ui_label(ui: *mut HostUiOpaque, text: *const i8, r: f32, g: f32, b: f32, a: f32);
    fn host_ui_separator(ui: *mut HostUiOpaque);
    fn host_ui_group_begin(ui: *mut HostUiOpaque, id: *const i8, h: f32) -> i32;
    fn host_ui_group_end(ui: *mut HostUiOpaque, id: *const i8, follow_tail: i32);
    fn host_ui_selectable(ui: *mut HostUiOpaque, label: *const i8, selected: i32) -> i32;
    fn host_ui_edit_focus(ui: *mut HostUiOpaque);
    fn host_ui_edit(ui: *mut HostUiOpaque, buf: *mut i8, cap: i32) -> i32;
    fn host_ui_edit_committed() -> i32;
    fn host_ui_bar(ui: *mut HostUiOpaque, n: i32, x0: *const f32, x1: *const f32, rgba: *const u8);
    fn host_ui_convert(
        ui: *mut HostUiOpaque,
        cmds: *mut HostUiCmd,
        max_cmds: i32,
        out: *mut HostUiDraw,
    ) -> i32;
    fn host_ui_finish(ui: *mut HostUiOpaque);
    fn host_ui_index_size() -> i32;
    fn host_ui_key_shift() -> i32;
    fn host_ui_key_ctrl() -> i32;
    fn host_ui_key_enter() -> i32;
    fn host_ui_key_tab() -> i32;
    fn host_ui_key_backspace() -> i32;
    fn host_ui_key_up() -> i32;
    fn host_ui_key_down_key() -> i32;
    fn host_ui_key_left() -> i32;
    fn host_ui_key_right() -> i32;
    fn host_ui_key_del() -> i32;
}

thread_local! {
    static CURRENT: Cell<*mut HostUiOpaque> = const { Cell::new(std::ptr::null_mut()) };
    static SCREEN: Cell<[f32; 2]> = const { Cell::new([1.0, 1.0]) };
}

fn current() -> *mut HostUiOpaque {
    CURRENT.with(|c| c.get())
}

pub struct UiKey;

impl UiKey {
    pub fn shift() -> i32 {
        unsafe { host_ui_key_shift() }
    }
    pub fn ctrl() -> i32 {
        unsafe { host_ui_key_ctrl() }
    }
    pub fn enter() -> i32 {
        unsafe { host_ui_key_enter() }
    }
    pub fn tab() -> i32 {
        unsafe { host_ui_key_tab() }
    }
    pub fn backspace() -> i32 {
        unsafe { host_ui_key_backspace() }
    }
    pub fn up() -> i32 {
        unsafe { host_ui_key_up() }
    }
    pub fn down() -> i32 {
        unsafe { host_ui_key_down_key() }
    }
    pub fn left() -> i32 {
        unsafe { host_ui_key_left() }
    }
    pub fn right() -> i32 {
        unsafe { host_ui_key_right() }
    }
    pub fn del() -> i32 {
        unsafe { host_ui_key_del() }
    }
}

#[derive(Clone)]
pub struct UiFrameInput {
    pub mouse: [f32; 2],
    pub buttons: [bool; 3],
    pub wheel: f32,
    pub text: String,
    pub keys: Vec<(i32, bool)>,
}

impl Default for UiFrameInput {
    fn default() -> Self {
        Self {
            mouse: [-1.0e6, -1.0e6],
            buttons: [false; 3],
            wheel: 0.0,
            text: String::new(),
            keys: Vec::new(),
        }
    }
}

pub struct HostUi {
    ui: *mut HostUiOpaque,
    pipeline: wgpu::RenderPipeline,
    font_bg: wgpu::BindGroup,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    vb_cap: u64,
    ib_cap: u64,
    screen_buf: wgpu::Buffer,
    input: UiFrameInput,
    screen: [f32; 2],
}

unsafe impl Send for HostUi {}

impl HostUi {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let ui = unsafe { host_ui_create() };
        assert!(!ui.is_null(), "nuklear context");
        let (pixels, w, h) = unsafe {
            let mut w = 0;
            let mut h = 0;
            let ptr = host_ui_atlas_pixels(ui, &mut w, &mut h);
            let n = (w as usize) * (h as usize) * 4;
            let bytes = if ptr.is_null() || n == 0 {
                vec![255u8; 4]
            } else {
                std::slice::from_raw_parts(ptr, n).to_vec()
            };
            (bytes, w.max(1) as u32, h.max(1) as u32)
        };
        unsafe { host_ui_atlas_end(ui) };

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("host-ui-font"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let font_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("host-ui-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("host-ui-samp"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let screen_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("host-ui-screen"),
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let font_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("host-ui-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&font_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: screen_buf.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("host-ui-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("host-ui-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("host-ui"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vb_cap = 256 * 1024;
        let ib_cap = 128 * 1024;
        let vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("host-ui-vb"),
            size: vb_cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("host-ui-ib"),
            size: ib_cap,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            ui,
            pipeline,
            font_bg,
            vb,
            ib,
            vb_cap,
            ib_cap,
            screen_buf,
            input: UiFrameInput::default(),
            screen: [1.0, 1.0],
        }
    }

    pub fn set_input(&mut self, input: UiFrameInput) {
        self.input = input;
    }

    pub fn begin_frame(&mut self, width: f32, height: f32) {
        self.screen = [width.max(1.0), height.max(1.0)];
        let ui = self.ui;
        let input = std::mem::take(&mut self.input);
        unsafe {
            host_ui_input_begin(ui);
            host_ui_motion(ui, input.mouse[0], input.mouse[1]);
            for (i, down) in input.buttons.iter().enumerate() {
                host_ui_button(
                    ui,
                    i as i32,
                    input.mouse[0],
                    input.mouse[1],
                    if *down { 1 } else { 0 },
                );
            }
            if input.wheel != 0.0 {
                host_ui_scroll(ui, input.wheel);
            }
            for (key, down) in &input.keys {
                host_ui_key(ui, *key, if *down { 1 } else { 0 });
            }
            for ch in input.text.chars() {
                host_ui_unicode(ui, ch as u32);
            }
            host_ui_input_end(ui);
        }
        CURRENT.with(|c| c.set(ui));
        SCREEN.with(|s| s.set(self.screen));
    }

    fn ensure_geom(&mut self, device: &wgpu::Device, vbytes: u64, ibytes: u64) {
        if vbytes > self.vb_cap {
            let cap = vbytes.next_power_of_two().max(self.vb_cap);
            self.vb = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("host-ui-vb"),
                size: cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vb_cap = cap;
        }
        if ibytes > self.ib_cap {
            let cap = ibytes.next_power_of_two().max(self.ib_cap);
            self.ib = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("host-ui-ib"),
                size: cap,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.ib_cap = cap;
        }
    }

    pub fn end_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        CURRENT.with(|c| c.set(std::ptr::null_mut()));
        let mut cmds = vec![
            HostUiCmd {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
                elem_count: 0,
                index_offset: 0
            };
            8192
        ];
        let mut draw = HostUiDraw {
            verts: std::ptr::null(),
            vert_stride: 20,
            vert_count: 0,
            indices: std::ptr::null(),
            index_count: 0,
        };
        let mut n =
            unsafe { host_ui_convert(self.ui, cmds.as_mut_ptr(), cmds.len() as i32, &mut draw) };
        if n == cmds.len() as i32 {
            cmds.resize(
                cmds.len() * 2,
                HostUiCmd {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                    elem_count: 0,
                    index_offset: 0,
                },
            );
            n = unsafe {
                host_ui_convert(self.ui, cmds.as_mut_ptr(), cmds.len() as i32, &mut draw)
            };
        }
        if n > 0 && !draw.verts.is_null() && draw.vert_count > 0 && draw.index_count > 0 {
            let vbytes = (draw.vert_count as usize) * (draw.vert_stride as usize);
            let ibytes = (draw.index_count as usize) * (unsafe { host_ui_index_size() } as usize);
            self.ensure_geom(device, vbytes as u64, ibytes as u64);
            {
                let verts = unsafe { std::slice::from_raw_parts(draw.verts, vbytes) };
                let indices = unsafe { std::slice::from_raw_parts(draw.indices, ibytes) };
                queue.write_buffer(&self.vb, 0, verts);
                queue.write_buffer(&self.ib, 0, indices);
                let screen = [self.screen[0], self.screen[1]];
                queue.write_buffer(&self.screen_buf, 0, bytemuck::bytes_of(&screen));
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("host-ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
                let _ = device;
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.font_bg, &[]);
                pass.set_vertex_buffer(0, self.vb.slice(..));
                let index_fmt = if unsafe { host_ui_index_size() } == 4 {
                    wgpu::IndexFormat::Uint32
                } else {
                    wgpu::IndexFormat::Uint16
                };
                pass.set_index_buffer(self.ib.slice(..ibytes as u64), index_fmt);
                let sw = self.screen[0] as u32;
                let sh = self.screen[1] as u32;
                for cmd in cmds.iter().take(n as usize) {
                    if cmd.elem_count <= 0 {
                        continue;
                    }
                    let x = cmd.x.max(0.0) as u32;
                    let y = cmd.y.max(0.0) as u32;
                    let w = cmd.w.max(0.0) as u32;
                    let h = cmd.h.max(0.0) as u32;
                    if x >= sw || y >= sh || w == 0 || h == 0 {
                        continue;
                    }
                    let w = w.min(sw.saturating_sub(x));
                    let h = h.min(sh.saturating_sub(y));
                    pass.set_scissor_rect(x, y, w, h);
                    let start = cmd.index_offset as u32;
                    let end = start + cmd.elem_count as u32;
                    pass.draw_indexed(start..end, 0, 0..1);
                }
            }
        }
        unsafe { host_ui_finish(self.ui) };
    }
}

impl Drop for HostUi {
    fn drop(&mut self) {
        unsafe { host_ui_destroy(self.ui) };
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_else(|_| CString::new("").unwrap())
}

pub fn begin_overlay(title: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    let t = cstr(title);
    unsafe { host_ui_begin(ui, t.as_ptr().cast(), x, y, w, h, host_ui_flags_overlay()) != 0 }
}

pub fn begin_panel(title: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    let t = cstr(title);
    unsafe { host_ui_begin(ui, t.as_ptr().cast(), x, y, w, h, host_ui_flags_panel()) != 0 }
}

pub fn begin_console(w: f32, h: f32) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    let t = cstr("##console");
    unsafe { host_ui_begin(ui, t.as_ptr().cast(), 0.0, 0.0, w, h, host_ui_flags_console()) != 0 }
}

pub fn end_window() {
    let ui = current();
    if !ui.is_null() {
        unsafe { host_ui_end(ui) };
    }
}

pub fn label(text: &str) {
    label_colored(text, [0.9, 0.9, 0.9, 1.0]);
}

pub fn label_colored(text: &str, rgba: [f32; 4]) {
    let ui = current();
    if ui.is_null() {
        return;
    }
    let t = cstr(text);
    unsafe {
        host_ui_layout_dynamic(ui, 14.0, 1);
        host_ui_label(ui, t.as_ptr().cast(), rgba[0], rgba[1], rgba[2], rgba[3]);
    }
}

pub fn separator() {
    let ui = current();
    if !ui.is_null() {
        unsafe { host_ui_separator(ui) };
    }
}

pub fn group_begin(id: &str, h: f32) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    let t = cstr(id);
    unsafe { host_ui_group_begin(ui, t.as_ptr().cast(), h) != 0 }
}

pub fn group_end(id: &str, follow_tail: bool) {
    let ui = current();
    if ui.is_null() {
        return;
    }
    let t = cstr(id);
    unsafe { host_ui_group_end(ui, t.as_ptr().cast(), if follow_tail { 1 } else { 0 }) };
}

pub fn selectable(label_text: &str, selected: bool) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    let t = cstr(label_text);
    unsafe {
        host_ui_layout_dynamic(ui, 18.0, 1);
        host_ui_selectable(ui, t.as_ptr().cast(), if selected { 1 } else { 0 }) != 0
    }
}

pub fn edit_focus() {
    let ui = current();
    if !ui.is_null() {
        unsafe { host_ui_edit_focus(ui) };
    }
}

pub fn edit_line(buf: &mut [u8]) -> bool {
    let ui = current();
    if ui.is_null() || buf.is_empty() {
        return false;
    }
    unsafe {
        host_ui_layout_dynamic(ui, 24.0, 1);
        let flags = host_ui_edit(ui, buf.as_mut_ptr() as *mut i8, buf.len() as i32);
        (flags & host_ui_edit_committed()) != 0
    }
}

pub fn key_pressed(key: i32) -> bool {
    let ui = current();
    if ui.is_null() {
        return false;
    }
    unsafe { host_ui_key_pressed(ui, key) != 0 }
}

pub fn bar(spans: &[(f32, f32, [u8; 4])]) {
    let ui = current();
    if ui.is_null() {
        return;
    }
    let mut x0 = Vec::with_capacity(spans.len());
    let mut x1 = Vec::with_capacity(spans.len());
    let mut rgba = Vec::with_capacity(spans.len() * 4);
    for (a, b, c) in spans {
        x0.push(*a);
        x1.push(*b);
        rgba.extend_from_slice(c);
    }
    unsafe {
        host_ui_layout_dynamic(ui, 14.0, 1);
        host_ui_bar(
            ui,
            spans.len() as i32,
            x0.as_ptr(),
            x1.as_ptr(),
            rgba.as_ptr(),
        );
    }
}

const WGSL: &str = r#"
struct VsIn {
  @location(0) pos: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) col: vec4<f32>,
};
struct VsOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) col: vec4<f32>,
};
struct Screen { size: vec2<f32> };
@group(0) @binding(2) var<uniform> screen: Screen;
@group(0) @binding(0) var font_tex: texture_2d<f32>;
@group(0) @binding(1) var font_samp: sampler;

@vertex
fn vs_main(in: VsIn) -> VsOut {
  var o: VsOut;
  let ndc = vec2<f32>(in.pos.x / screen.size.x * 2.0 - 1.0, 1.0 - in.pos.y / screen.size.y * 2.0);
  o.clip = vec4<f32>(ndc, 0.0, 1.0);
  o.uv = in.uv;
  o.col = in.col;
  return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let t = textureSample(font_tex, font_samp, in.uv);
  return vec4<f32>(in.col.rgb, in.col.a * t.a);
}
"#;

pub fn screen() -> [f32; 2] {
    SCREEN.with(|s| s.get())
}
