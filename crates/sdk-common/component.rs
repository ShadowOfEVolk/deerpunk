use serde_json::Value;

/// Hook for `#[game_component(prefab)]`.
pub trait PrefabFromValue: Sized {
    fn from_prefab_value(value: &Value) -> Self;
}

/// Manual inspect schema when not using `#[derive(JsonSchema)]` + `#[game_component(inspect)]`.
pub trait ComponentInspect: Sized {
    fn inspect_schema_cbor() -> &'static [u8];
    fn to_inspect_json(&self) -> Value;
}

/// Hook for `#[game_component(inspect, schema = …)]` — schema comes from the attribute.
pub trait InspectDump: Sized {
    fn to_inspect_json(&self) -> Value;
}
