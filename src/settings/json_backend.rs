use std::path::PathBuf;

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct JsonBackend {
    #[serde(default)]
    pub file_path: Option<PathBuf>,
}

/// Return the fixed path to entries.json in your desired folder
pub fn get_default_json_path() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from("/Users/nunezcha/Documents/log_cold_storage/entries.json"))
}
