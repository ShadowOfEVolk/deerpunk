use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CollectionMode {
    List,
    Get,
    Stream,
    Catalog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CollectionGroup {
    Runtime,
    Definitions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDescriptor {
    pub id: String,
    pub label: String,
    pub group: CollectionGroup,
    pub modes: Vec<CollectionMode>,
    pub id_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nested_in: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub proto: i64,
    pub client: String,
    #[serde(default)]
    pub client_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub proto: i64,
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    pub collections: Vec<CollectionDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComponentSnapshot {
    Tag {
        slug: String,
        kind: TagKind,
    },
    Data {
        slug: String,
        data: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TagKind {
    Tag,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityListItem {
    pub index: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityListResult {
    pub time: f32,
    pub count: usize,
    pub entities: Vec<EntityListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityGetResult {
    pub index: u64,
    pub name: String,
    pub components: Vec<ComponentSnapshot>,
}

pub const RPC_INITIALIZE: &str = "inspector.initialize";
pub const RPC_INITIALIZED: &str = "inspector.initialized";
pub const RPC_ENTITIES_LIST: &str = "entities.list";
pub const RPC_ENTITIES_GET: &str = "entities.get";
