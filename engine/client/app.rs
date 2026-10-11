use std::path::Path;

use deerpunk_engine_common::input::Input;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use sdl2::event::Event;
use sdl2::pixels::PixelFormatEnum;
use sdl2::surface::Surface;
use sdl2::video::Window;

pub struct SendableWindowHandle {
    raw_window: RawWindowHandle,
    raw_display: Option<RawDisplayHandle>,
}

unsafe impl Send for SendableWindowHandle {}
unsafe impl Sync for SendableWindowHandle {}

impl HasWindowHandle for SendableWindowHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.raw_window) })
    }
}

impl HasDisplayHandle for SendableWindowHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        match self.raw_display {
            Some(d) => Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(d) }),
            None => Err(raw_window_handle::HandleError::Unavailable),
        }
    }
}

pub struct App {
    window: Window,
    _sdl: sdl2::Sdl,
    video: sdl2::VideoSubsystem,
    event_pump: sdl2::EventPump,
    text_input_active: bool,
}

impl App {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        let sdl = sdl2::init().expect("SDL2 init failed");
        let video = sdl.video().expect("SDL2 video init failed");
        let event_pump = sdl.event_pump().expect("SDL2 event pump failed");

        let _mouse = sdl.mouse();
        // mouse.enable_relative_mouse_mode(true);

        let window = video
            .window(title, width, height)
            .build()
            .expect("Failed to create SDL2 window");

        // let _ = window.set_mouse_grab(true);

        Self {
            window,
            _sdl: sdl,
            video,
            event_pump,
            text_input_active: false,
        }
    }

    pub fn set_text_input_active(&mut self, active: bool) {
        if active == self.text_input_active {
            return;
        }
        self.text_input_active = active;
        if active {
            self.video.text_input().start();
        } else {
            self.video.text_input().stop();
        }
    }

    pub fn poll_events(&mut self, input: &mut Input) -> bool {
        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => return false,
                Event::Window { win_event, .. } => match win_event {
                    sdl2::event::WindowEvent::Close => return false,
                    _ => {}
                },
                Event::KeyDown {
                    scancode: Some(scancode),
                    repeat: false,
                    ..
                } => {
                    input.set_key(scancode as u16, true);
                }
                Event::KeyUp {
                    scancode: Some(scancode),
                    repeat: false,
                    ..
                } => {
                    input.set_key(scancode as u16, false);
                }
                Event::TextInput { text, .. } => {
                    input.push_text(&text);
                }
                Event::MouseMotion { x, y, xrel, yrel, .. } => {
                    input.add_mouse_delta(xrel as f32, yrel as f32);
                    input.set_mouse_position(x as f32, y as f32);
                }
                Event::MouseButtonDown { mouse_btn, .. } => {
                    let bit = match mouse_btn {
                        sdl2::mouse::MouseButton::Left => 1u8,
                        sdl2::mouse::MouseButton::Right => 2,
                        sdl2::mouse::MouseButton::Middle => 4,
                        _ => 0,
                    };
                    input.set_mouse_buttons(input.mouse_buttons() | bit);
                }
                Event::MouseButtonUp { mouse_btn, .. } => {
                    let bit = match mouse_btn {
                        sdl2::mouse::MouseButton::Left => 1u8,
                        sdl2::mouse::MouseButton::Right => 2,
                        sdl2::mouse::MouseButton::Middle => 4,
                        _ => 0,
                    };
                    input.set_mouse_buttons(input.mouse_buttons() & !bit);
                }
                _ => {}
            }
        }
        true
    }

    pub fn create_wgpu_surface(&self, instance: &wgpu::Instance) -> wgpu::Surface<'static> {
        let raw_window = self.window.window_handle().unwrap().as_raw();
        let raw_display = self.window.display_handle().ok().map(|d| d.as_raw());

        let handle = Box::new(SendableWindowHandle {
            raw_window,
            raw_display,
        });

        instance
            .create_surface(wgpu::SurfaceTarget::DisplayAndWindow(handle))
            .expect("Failed to create wgpu surface from SDL2")
    }

    pub fn window_size(&self) -> (u32, u32) {
        self.window.size()
    }

    pub fn set_title(&mut self, title: &str) {
        if let Err(e) = self.window.set_title(title) {
            log::warn!("[app] set_title: {e}");
        }
    }

    /// Load ICO/PNG/BMP via `image` and set as the window icon.
    pub fn set_icon_from_path(&mut self, path: &Path) {
        match load_icon_surface(path) {
            Ok(surface) => self.window.set_icon(surface),
            Err(e) => log::warn!("[app] icon {}: {e}", path.display()),
        }
    }
}

fn load_icon_surface(path: &Path) -> Result<Surface<'static>, String> {
    let rgba = image::open(path)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut owned = Surface::new(w, h, PixelFormatEnum::RGBA32).map_err(|e| e.to_string())?;
    let pitch = owned.pitch() as usize;
    let src = rgba.as_raw();
    let row_bytes = (w as usize) * 4;
    owned.with_lock_mut(|dst| {
        for y in 0..h as usize {
            let s = y * row_bytes;
            let d = y * pitch;
            dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
        }
    });
    Ok(owned)
}
