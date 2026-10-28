// Component type identifiers for network replication
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentType {
    Transform = 0,
    SimpleCircleRenderer = 1,
}

impl ComponentType {
    pub fn from_id(id: u16) -> Option<Self> {
        match id {
            0 => Some(ComponentType::Transform),
            1 => Some(ComponentType::SimpleCircleRenderer),
            _ => None,
        }
    }
    
    pub fn to_id(self) -> u16 {
        self as u16
    }
}

