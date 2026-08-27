use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use sdl2::event::Event;
use sdl2::video::Window;

pub struct SendableWindowHandle {
    raw_window: RawWindowHandle,
    raw_display: Option<RawDisplayHandle>,
}

unsafe impl Send for SendableWindowHandle {}
unsafe impl Sync for SendableWindowHandle {}

impl HasWindowHandle for SendableWindowHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.raw_window) })
    }
}

impl HasDisplayHandle for SendableWindowHandle {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        match self.raw_display {
            Some(d) => Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(d) }),
            None => Err(raw_window_handle::HandleError::Unavailable),
        }
    }
}

pub struct App {
    window: Window,
    _sdl: sdl2::Sdl,
    _video: sdl2::VideoSubsystem,
    event_pump: sdl2::EventPump,
}

impl App {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        let sdl = sdl2::init().expect("SDL2 init failed");
        let video = sdl.video().expect("SDL2 video init failed");
        let event_pump = sdl.event_pump().expect("SDL2 event pump failed");

        let window = video
            .window(title, width, height)
            .build()
            .expect("Failed to create SDL2 window");

        Self {
            window,
            _sdl: sdl,
            _video: video,
            event_pump,
        }
    }

    pub fn poll_events(&mut self) -> bool {
        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => return false,
                Event::Window { win_event, .. } => match win_event {
                    sdl2::event::WindowEvent::Close => return false,
                    _ => {}
                },
                _ => {}
            }
        }
        true
    }

    pub fn create_wgpu_surface(
        &self,
        instance: &wgpu::Instance,
    ) -> wgpu::Surface<'static> {
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
}
