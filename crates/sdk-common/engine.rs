//! Engine-owned component layouts still used by gamelib (until fully migrated).

pub use deerpunk_engine_common::components::{LuaScript, MapKv, TransformComponent};

pub mod physics {
    pub use deerpunk_engine_common::physics::*;
}
