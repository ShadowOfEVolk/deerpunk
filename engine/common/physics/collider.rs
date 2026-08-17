use glam::Vec3;

/// Категории коллизий (битовые маски для box3d QueryFilter).
pub mod categories {
    pub const TERRAIN: u16 = 1 << 0;
    pub const TANK: u16 = 1 << 1;
    pub const PROJECTILE: u16 = 1 << 2;
    pub const SENSOR: u16 = 1 << 3;
}

/// Форма коллайдера.
#[derive(Clone, Debug)]
pub enum ColliderShape {
    /// AABB: half-extents по X, Y, Z.
    Box(Vec3),
    /// Бесконечная плоскость (Z-up мир): нормаль + расстояние от начала координат.
    HalfSpace {
        normal: Vec3,
        offset: f32,
    },
    /// Набор дочерних форм (смещение + shape).
    Compound(Vec<(Vec3, ColliderShape)>),
}

/// Тип тела.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyType {
    /// Статическое (стены, кирпичи, пол).
    Static,
    /// Динамическое (танки, снаряды).
    Dynamic,
    /// Кинематическое (движется скриптом, не физикой).
    Kinematic,
}

/// ECS-компонент: физический коллайдер.
#[derive(Clone, Debug)]
pub struct Collider {
    pub shape: ColliderShape,
    pub body_type: BodyType,
    pub category_bits: u16,
    pub mask_bits: u16,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
}

impl Collider {
    pub fn static_box(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Static,
            category_bits: categories::TERRAIN,
            mask_bits: categories::TANK | categories::PROJECTILE,
            friction: 0.5,
            restitution: 0.0,
            density: 0.0,
        }
    }

    pub fn dynamic_box(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Dynamic,
            category_bits: categories::TANK,
            mask_bits: categories::TERRAIN | categories::TANK | categories::SENSOR,
            friction: 0.3,
            restitution: 0.1,
            density: 1.0,
        }
    }

    pub fn sensor(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Static,
            category_bits: categories::SENSOR,
            mask_bits: categories::TANK,
            friction: 0.0,
            restitution: 0.0,
            density: 0.0,
        }
    }

    pub fn half_space(normal: Vec3, offset: f32) -> Self {
        Self {
            shape: ColliderShape::HalfSpace { normal, offset },
            body_type: BodyType::Static,
            category_bits: categories::TERRAIN,
            mask_bits: categories::TANK | categories::PROJECTILE,
            friction: 0.5,
            restitution: 0.0,
            density: 0.0,
        }
    }
}

/// Маркер-компонент: тело является сенсором (триггер, без физического ответа).
#[derive(Clone, Copy, Debug, Default)]
pub struct Sensor;
