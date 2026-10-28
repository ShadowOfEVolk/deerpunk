//! Raw encoded audio loaded through [`FileLoader`] (`type_name` = `"audio"`).

#[derive(Clone)]
pub struct AudioClip {
    pub bytes: Vec<u8>,
    pub extension: String,
}
