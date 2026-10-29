//! Single-level parent link (pawn → view camera child).

use crate::world::EntityId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParentComponent {
    pub parent: EntityId,
}
