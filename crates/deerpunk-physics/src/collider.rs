//! Legacy helpers / markers. ECS [`Collider`] POD lives in [`super::abi`].

pub use super::abi::Collider;

/// Маркер-компонент: тело является сенсором (триггер, без физического ответа).
#[derive(Clone, Copy, Debug, Default)]
pub struct Sensor;
