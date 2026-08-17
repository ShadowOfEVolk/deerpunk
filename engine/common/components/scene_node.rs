use hecs::Entity;

#[derive(Clone, Debug)]
pub struct SceneNode {
    pub name: String,
    pub parent: Option<Entity>,
}
