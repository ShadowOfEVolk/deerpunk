pub mod app;
pub mod audio;
pub mod cmd;
pub mod cvar;
pub mod physics;
pub mod renderer;
pub mod resources;
pub mod version;
pub mod world;

pub trait EngineApi {
    fn app(&self) -> &dyn app::AppApi;
    fn renderer(&self) -> &dyn renderer::RendererApi;
    fn audio(&self) -> &dyn audio::AudioApi;
    fn physics(&self) -> &dyn physics::PhysicsApi;
    fn world(&self) -> &dyn world::WorldApi;
    fn cmd(&self) -> &dyn cmd::CmdApi;
    fn cvar(&self) -> &dyn cvar::CvarApi;
    fn resources(&self) -> &dyn resources::ResourcesApi;
}
