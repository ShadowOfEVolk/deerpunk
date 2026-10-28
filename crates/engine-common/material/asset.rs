//! Material asset — serializable instance of a shader.
//!
//! JSON for authoring, CBOR for pack/network. References a shader by path
//! (asset, not code). Properties validated against `ShaderMeta` at load time.
//!
//! ```json
//! {
//!   "shader": "file://shaders/pbr.wgsl",
//!   "shader_defines": ["ALPHA_MASK"],
//!   "properties": {
//!     "_BaseColor": [1, 1, 1, 1],
//!     "_Metallic": 0.0,
//!     "_Roughness": 0.5,
//!     "_BaseMap": "file://textures/wood.png"
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};

/// A material asset — shader reference + property values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialAsset {
    /// Shader path, e.g. `"file://shaders/pbr.wgsl"`. Resolved via ResourceSystem.
    pub shader: String,
    /// Preprocessor defines for ubershader variants, e.g. `["ALPHA_MASK"]`.
    #[serde(default)]
    pub shader_defines: Vec<String>,
    /// Property values, keyed by name (must match `ShaderMeta.properties`).
    ///
    /// Serialized as a JSON object (map) for authoring readability, but the
    /// deserializer also accepts an array of `[name, value]` pairs for
    /// programmatic/compact formats.
    #[serde(default, with = "props_map")]
    pub properties: Vec<(String, PropertyValueSer)>,
}

/// Serializable property value. Tagged enum — JSON/CBOR friendly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PropertyValueSer {
    Float(f32),
    Color([f32; 4]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    /// Texture path: `"file://textures/wood.png"` or fallback name `"white"`.
    Texture(String),
}

impl PropertyValueSer {
    /// Check if this value is compatible with the given property kind.
    pub fn matches_kind(&self, kind: crate::material::shader_meta::PropertyKind) -> bool {
        use crate::material::shader_meta::PropertyKind as K;
        matches!(
            (self, kind),
            (Self::Float(_), K::Float)
                | (Self::Color(_), K::Color)
                | (Self::Vec3(_), K::Vec3)
                | (Self::Vec4(_), K::Vec4)
                | (Self::Texture(_), K::Texture2d)
                | (Self::Texture(_), K::TextureCube)
        )
    }
}

// ── props_map: serde helper for Vec<(String, PropertyValueSer)> ───────
//
// Serializes as a map (JSON object) for authoring readability; deserializes
// from either a map or an array of [name, value] pairs.

mod props_map {
    use serde::de::{MapAccess, SeqAccess, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    use super::PropertyValueSer;

    pub fn serialize<S>(v: &[(String, PropertyValueSer)], s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(v.len()))?;
        for (k, val) in v {
            map.serialize_entry(k, val)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Vec<(String, PropertyValueSer)>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PropsVisitor;

        impl<'de> Visitor<'de> for PropsVisitor {
            type Value = Vec<(String, PropertyValueSer)>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a map or an array of [name, value] pairs")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, PropertyValueSer>()? {
                    out.push((k, v));
                }
                Ok(out)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut out = Vec::new();
                while let Some(pair) = seq.next_element::<(String, PropertyValueSer)>()? {
                    out.push(pair);
                }
                Ok(out)
            }
        }

        d.deserialize_any(PropsVisitor)
    }
}

impl MaterialAsset {
    /// Parse from JSON.
    pub fn from_json(data: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(data)
    }

    /// Serialize to JSON (pretty, for authoring).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parse from CBOR bytes.
    pub fn from_cbor(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let mat: Self = ciborium::de::from_reader(data)?;
        Ok(mat)
    }

    /// Serialize to CBOR bytes (compact, for pack/network).
    pub fn to_cbor(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf)?;
        Ok(buf)
    }

    /// Look up a property value by name.
    pub fn get(&self, name: &str) -> Option<&PropertyValueSer> {
        self.properties
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// BSP `#material:…` assets sometimes assign `file://…#lightmap:…` to `_BaseMap`.
    /// Lightmaps must use [`_Lightmap`](Self::get) — move misplaced URIs before GPU resolve.
    pub fn normalize_lightmap_texture_props(&mut self) {
        let has_lightmap = self.properties.iter().any(|(n, _)| n == "_Lightmap");
        let mut stolen: Vec<String> = Vec::new();
        self.properties.retain(|(name, value)| {
            if let PropertyValueSer::Texture(uri) = value {
                if uri.contains("#lightmap:") && name != "_Lightmap" {
                    if !stolen.iter().any(|u| u == uri) {
                        stolen.push(uri.clone());
                    }
                    return false;
                }
            }
            true
        });
        if !has_lightmap {
            for uri in stolen {
                self.properties
                    .push(("_Lightmap".into(), PropertyValueSer::Texture(uri)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let mat = MaterialAsset {
            shader: "file://shaders/pbr.wgsl".into(),
            shader_defines: vec!["ALPHA_MASK".into()],
            properties: vec![
                (
                    "_BaseColor".into(),
                    PropertyValueSer::Color([1.0, 1.0, 1.0, 1.0]),
                ),
                ("_Metallic".into(), PropertyValueSer::Float(0.0)),
                (
                    "_BaseMap".into(),
                    PropertyValueSer::Texture("file://textures/wood.png".into()),
                ),
            ],
        };
        let json = mat.to_json_pretty().unwrap();
        let back = MaterialAsset::from_json(&json).unwrap();
        assert_eq!(back.shader, mat.shader);
        assert_eq!(back.shader_defines, mat.shader_defines);
        assert_eq!(back.properties.len(), 3);
    }

    #[test]
    fn cbor_roundtrip() {
        let mat = MaterialAsset {
            shader: "file://shaders/pbr.wgsl".into(),
            shader_defines: vec![],
            properties: vec![("_Roughness".into(), PropertyValueSer::Float(0.5))],
        };
        let cbor = mat.to_cbor().unwrap();
        let back = MaterialAsset::from_cbor(&cbor).unwrap();
        assert_eq!(back.shader, mat.shader);
        assert_eq!(back.properties.len(), 1);
    }

    #[test]
    fn bsp_lightmap_on_base_map_moved() {
        let mut mat = MaterialAsset {
            shader: "file://shaders/mesh.wgsl".into(),
            shader_defines: vec![],
            properties: vec![(
                "_BaseMap".into(),
                PropertyValueSer::Texture("file://maps/oldlifehome.bsp#lightmap:world".into()),
            )],
        };
        mat.normalize_lightmap_texture_props();
        assert!(mat.get("_BaseMap").is_none());
        assert_eq!(
            mat.get("_Lightmap"),
            Some(&PropertyValueSer::Texture(
                "file://maps/oldlifehome.bsp#lightmap:world".into()
            ))
        );
    }

    #[test]
    fn minimal_material() {
        let json = r#"{"shader": "file://shaders/unlit.wgsl"}"#;
        let mat = MaterialAsset::from_json(json).unwrap();
        assert_eq!(mat.shader, "file://shaders/unlit.wgsl");
        assert!(mat.shader_defines.is_empty());
        assert!(mat.properties.is_empty());
    }

    /// The authoring format: properties as a JSON object (map).
    #[test]
    fn json_map_properties() {
        let json = r#"{
            "shader": "file://shaders/mesh.wgsl",
            "properties": {
                "_BaseColor": [1.0, 1.0, 1.0, 1.0],
                "_Metallic": 0.0,
                "_Roughness": 0.9,
                "_BaseMap": "slot:42"
            }
        }"#;
        let mat = MaterialAsset::from_json(json).unwrap();
        assert_eq!(mat.shader, "file://shaders/mesh.wgsl");
        assert_eq!(mat.properties.len(), 4);
        assert_eq!(mat.get("_Roughness"), Some(&PropertyValueSer::Float(0.9)));
        assert_eq!(
            mat.get("_BaseMap"),
            Some(&PropertyValueSer::Texture("slot:42".into()))
        );
        // Round-trip back through JSON (serializes as a map).
        let back = mat.to_json_pretty().unwrap();
        assert!(back.contains("\"_BaseMap\""));
        let mat2 = MaterialAsset::from_json(&back).unwrap();
        assert_eq!(mat2.properties.len(), 4);
    }

    /// Compact format: properties as an array of [name, value] pairs.
    #[test]
    fn json_array_properties() {
        let json = r#"{
            "shader": "file://shaders/mesh.wgsl",
            "properties": [["_Roughness", 0.5], ["_Metallic", 0.1]]
        }"#;
        let mat = MaterialAsset::from_json(json).unwrap();
        assert_eq!(mat.properties.len(), 2);
        assert_eq!(mat.get("_Roughness"), Some(&PropertyValueSer::Float(0.5)));
    }
}
