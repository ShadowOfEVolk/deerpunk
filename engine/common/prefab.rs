use std::collections::HashMap;
use std::sync::Arc;

use hecs::World;
use handlebars::Handlebars;
use serde_json::Value;

use crate::components::SceneNode;

/// Describes one component type that can appear in a prefab file.
pub struct PrefabEntry {
    pub type_name: &'static str,
    pub deserialize: fn(&Value) -> Box<dyn FnOnce(&mut World, hecs::Entity)>,
}

inventory::collect!(PrefabEntry);

/// Registry: maps type_name → PrefabEntry, built once from inventory.
pub struct PrefabRegistry {
    factories: HashMap<&'static str, &'static PrefabEntry>,
}

impl PrefabRegistry {
    pub fn new() -> Self {
        let mut factories = HashMap::new();
        for entry in inventory::iter::<PrefabEntry> {
            factories.insert(entry.type_name, entry);
        }
        Self { factories }
    }

    pub fn has_type(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }

    /// Spawn a single entity from a "components" array (already interpolated JSON).
    fn spawn_entity(
        &self,
        world: &mut World,
        components: &[Value],
    ) -> hecs::Entity {
        let entity = world.spawn(());
        for comp_val in components {
            let type_name = match comp_val["type"].as_str() {
                Some(t) => t,
                None => continue,
            };
            let data = &comp_val["data"];
            if let Some(factory) = self.factories.get(type_name) {
                let apply = (factory.deserialize)(data);
                apply(world, entity);
            } else {
                eprintln!("[prefab] unknown component type: {type_name}");
            }
        }
        entity
    }
}

impl Default for PrefabRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached pre-compiled prefab template.
pub struct PrefabTemplate {
    pub name: String,
    template_string: Arc<String>,
}

impl PrefabTemplate {
    pub fn load(path: &str) -> Self {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("[prefab] cannot read {path}: {e}"));
        Self {
            name: path.to_owned(),
            template_string: Arc::new(content),
        }
    }

    pub fn template_str(&self) -> &str {
        &self.template_string
    }
}

/// A loaded and interpolated prefab scene, ready to spawn.
pub struct PrefabScene {
    pub root_json: Value,
}

impl PrefabScene {
    /// Render handlebars template with variables → parse JSON.
    pub fn from_template(template: &PrefabTemplate, variables: &HashMap<String, String>) -> Self {
        let mut hbs = Handlebars::new();
        hbs.register_template_string("prefab", template.template_str())
            .expect("[prefab] failed to register template");

        let rendered = hbs.render("prefab", variables)
            .expect("[prefab] failed to render template");

        let root_json: Value = serde_json::from_str(&rendered)
            .expect("[prefab] failed to parse rendered JSON");

        Self { root_json }
    }

    /// Parse a pre-rendered JSON string directly (no handlebars).
    pub fn from_json(json_str: &str) -> Self {
        let root_json: Value = serde_json::from_str(json_str)
            .expect("[prefab] failed to parse JSON");
        Self { root_json }
    }

    /// Spawn all entities from this scene into the world.
    /// Returns spawned entities in order, with SceneNode parent linking.
    pub fn spawn(&self, registry: &PrefabRegistry, world: &mut World) -> Vec<hecs::Entity> {
        let entities_val = match self.root_json.get("entities") {
            Some(Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };

        let mut name_map: HashMap<String, hecs::Entity> = HashMap::new();
        let mut deferred: Vec<(String, Option<String>, Vec<Value>)> = Vec::new();

        for ent in entities_val {
            let name = ent["name"].as_str()
                .unwrap_or("unnamed")
                .to_owned();
            let parent = ent["parent"].as_str().map(|s| s.to_owned());
            let components = ent["components"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            let entity = world.spawn(());
            name_map.insert(name.clone(), entity);
            deferred.push((name, parent, components));
        }

        for (name, parent_name, components) in deferred {
            let entity = name_map[&name];

            for comp_val in &components {
                let type_name = match comp_val["type"].as_str() {
                    Some(t) => t,
                    None => continue,
                };
                let data = &comp_val["data"];
                if let Some(factory) = registry.factories.get(type_name) {
                    let apply = (factory.deserialize)(data);
                    apply(world, entity);
                } else {
                    eprintln!("[prefab] unknown component type: {type_name}");
                }
            }

            let parent_entity = parent_name
                .and_then(|p| name_map.get(&p))
                .copied();

            let _ = world.insert_one(entity, SceneNode {
                name,
                parent: parent_entity,
            });
        }

        name_map.values().copied().collect()
    }
}

/// Convenience: load + interpolate + spawn in one call.
pub fn load_prefab(
    registry: &PrefabRegistry,
    world: &mut World,
    template: &PrefabTemplate,
    variables: &HashMap<String, String>,
) -> Vec<hecs::Entity> {
    let scene = PrefabScene::from_template(template, variables);
    scene.spawn(registry, world)
}

/// Macro for components to register themselves with the prefab system.
#[macro_export]
macro_rules! register_prefab {
    ($type_name:expr, $component:ty) => {
        inventory::submit! {
            $crate::prefab::PrefabEntry {
                type_name: $type_name,
                deserialize: |val| {
                    let comp: $component = serde_json::from_value(val.clone())
                        .expect(concat!("failed to deserialize prefab component: ", $type_name));
                    Box::new(move |world, entity| {
                        world.insert_one(entity, comp).ok();
                    })
                },
            }
        }
    };
}
