//! wgpu instance handles. Surface, depth targets, and passes live on [`crate::renderer::Renderer`].

/// Adapter, device, and queue for one renderer.
///
/// Swapchain surface and depth textures stay on the renderer: they follow the window
/// and the camera targets, not the device lifetime.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// `INDIRECT_EXECUTION` and `INDIRECT_FIRST_INSTANCE`. Without both, mesh multi-draw stays off.
    pub multi_draw_indirect: bool,
    /// Non-zero `first_instance` in indirect draws. False means the device would ignore the slot.
    pub indirect_first_instance: bool,
}

impl Gpu {
    /// Pick an adapter compatible with `compatible_surface` when one is given.
    pub fn new(instance: &wgpu::Instance, compatible_surface: Option<&wgpu::Surface<'_>>) -> Self {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface,
            ..Default::default()
        }))
        .expect("no adapter for wgpu");
        let adapter_features = adapter.features();
        let mut required_features = wgpu::Features::empty();
        required_features |= wgpu::Features::POLYGON_MODE_LINE;
        if adapter_features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
            required_features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            ..Default::default()
        }))
        .expect("no device for wgpu");
        let indirect_first_instance = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        let downlevel_indirect = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
        if !indirect_first_instance {
            log::warn!(
                "GPU/device missing INDIRECT_FIRST_INSTANCE — mesh MDI disabled (indirect first_instance would be ignored)"
            );
        }
        Self {
            adapter,
            device,
            queue,
            multi_draw_indirect: downlevel_indirect && indirect_first_instance,
            indirect_first_instance,
        }
    }
}
