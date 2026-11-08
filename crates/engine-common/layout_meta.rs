pub use deerpunk_engine_world::layout_meta::*;

pub fn transform_component_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::TransformComponent;

    LayoutMeta {
        name: "TransformComponent".into(),
        size: size_of::<TransformComponent>() as u32,
        align: align_of::<TransformComponent>() as u32,
        role: ComponentRole::Replicated,
        fields: vec![
            FieldMeta {
                id: 0,
                name: "position".into(),
                offset: offset_of!(TransformComponent, position) as u32,
                size: 12,
                align: 4,
                wire: WireKind::Vec3,
                flags: FieldFlags::REPLICATE_HOT,
                doc: Some("World position (Z-up)".into()),
            },
            FieldMeta {
                id: 1,
                name: "rotation".into(),
                offset: offset_of!(TransformComponent, rotation) as u32,
                size: 16,
                align: 4,
                wire: WireKind::Quat,
                flags: FieldFlags::REPLICATE_HOT,
                doc: None,
            },
            FieldMeta {
                id: 2,
                name: "scale".into(),
                offset: offset_of!(TransformComponent, scale) as u32,
                size: 12,
                align: 4,
                wire: WireKind::Vec3,
                flags: FieldFlags::REPLICATE_COLD,
                doc: None,
            },
        ],
    }
}

pub fn worldspawn_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::Worldspawn;

    LayoutMeta {
        name: "Worldspawn".into(),
        size: size_of::<Worldspawn>() as u32,
        align: align_of::<Worldspawn>() as u32,
        role: ComponentRole::Replicated,
        fields: vec![FieldMeta {
            id: 0,
            name: "map".into(),
            offset: offset_of!(Worldspawn, map) as u32,
            size: 64,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD,
            doc: Some("BSP map name".into()),
        }],
    }
}

pub fn draw_material_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::material::DrawMaterial;

    LayoutMeta {
        name: "DrawMaterial".into(),
        size: size_of::<DrawMaterial>() as u32,
        align: align_of::<DrawMaterial>() as u32,
        role: ComponentRole::Replicated,
        fields: vec![FieldMeta {
            id: 0,
            name: "uri".into(),
            offset: offset_of!(DrawMaterial, uri) as u32,
            size: crate::material::component::DRAW_MATERIAL_URI_MAX as u32,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD | FieldFlags::REPLICATE_HOT,
            doc: Some("Material resource URI (server authoritative)".into()),
        }],
    }
}

pub fn mesh3d_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::Mesh3D;

    LayoutMeta {
        name: "Mesh3D".into(),
        size: size_of::<Mesh3D>() as u32,
        align: align_of::<Mesh3D>() as u32,
        role: ComponentRole::Replicated,
        fields: vec![FieldMeta {
            id: 0,
            name: "uri".into(),
            offset: offset_of!(Mesh3D, uri) as u32,
            size: crate::components::mesh3d::MESH3D_URI_MAX as u32,
            align: 1,
            wire: WireKind::Indirect,
            flags: FieldFlags::REPLICATE_COLD | FieldFlags::REPLICATE_HOT,
            doc: Some("Mesh resource URI (server authoritative)".into()),
        }],
    }
}

/// Built-in layout for engine `StudioModel`.
pub fn studio_model_layout() -> LayoutMeta {
    use std::mem::{align_of, offset_of, size_of};

    use crate::components::StudioModel;

    LayoutMeta {
        name: "StudioModel".into(),
        size: size_of::<StudioModel>() as u32,
        align: align_of::<StudioModel>() as u32,
        role: ComponentRole::Replicated,
        fields: vec![
            FieldMeta {
                id: 0,
                name: "body".into(),
                offset: offset_of!(StudioModel, body) as u32,
                size: 4,
                align: 4,
                wire: WireKind::U32,
                flags: FieldFlags::PREFAB | FieldFlags::INSPECT | FieldFlags::REPLICATE_HOT,
                doc: Some("Bodygroup variant".into()),
            },
            FieldMeta {
                id: 1,
                name: "skin".into(),
                offset: offset_of!(StudioModel, skin) as u32,
                size: 4,
                align: 4,
                wire: WireKind::U32,
                flags: FieldFlags::PREFAB | FieldFlags::INSPECT | FieldFlags::REPLICATE_HOT,
                doc: Some("Skin index".into()),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_studio_model() {
        let meta = studio_model_layout();
        let bytes = encode(&meta);
        let back = decode(&bytes).expect("decode");
        assert_eq!(back, meta);
        assert!(back.resolve_path("StudioModel.body").is_some());
        assert!(back.resolve_path("body").is_some());
    }

    #[test]
    fn transform_component_layout_offsets() {
        use std::mem::{offset_of, size_of};

        use crate::components::TransformComponent;

        let meta = transform_component_layout();
        assert_eq!(meta.size as usize, size_of::<TransformComponent>());
        let pos = meta.field("position").expect("position");
        let rot = meta.field("rotation").expect("rotation");
        let scale = meta.field("scale").expect("scale");
        assert_eq!(pos.offset, offset_of!(TransformComponent, position) as u32);
        assert_eq!(rot.offset, offset_of!(TransformComponent, rotation) as u32);
        assert_eq!(scale.offset, offset_of!(TransformComponent, scale) as u32);
        // Vec3 (12) + align Quat to 16 → rotation at 16, scale at 32 on typical targets.
        assert_eq!(rot.offset, 16);
        assert_eq!(scale.offset, 32);
    }

    #[test]
    fn json_schema_has_properties() {
        let meta = studio_model_layout();
        let schema = layout_to_json_schema(&meta);
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(props.contains_key("body"));
        assert!(props.contains_key("skin"));
    }
}
