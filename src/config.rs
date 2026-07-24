use std::path::PathBuf;

use crate::api::MetadataSourceKind;

/// Configuration struct that holds all settings for m4b-merge
#[derive(Debug, Clone)]
pub struct Config {
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub api_url: Option<String>,
    pub metadata_source: MetadataSourceKind,
    pub completed_directory: Option<PathBuf>,
    pub num_cpus: usize,
    pub log_level: String,
    pub path_format: String,
    pub dry_run: bool,
    pub metadata_id: Option<String>,
}

impl Config {
    /// Create a new Config from CLI arguments
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        inputs: Vec<PathBuf>,
        output: Option<PathBuf>,
        api_url: Option<String>,
        metadata_source: MetadataSourceKind,
        completed_directory: Option<PathBuf>,
        num_cpus: usize,
        log_level: String,
        path_format: String,
        dry_run: bool,
        metadata_id: Option<String>,
    ) -> Self {
        Self {
            inputs,
            output,
            api_url,
            metadata_source,
            completed_directory,
            num_cpus,
            log_level,
            path_format,
            dry_run,
            metadata_id,
        }
    }
}

impl Config {
    /// Validate configuration and return an error message if invalid
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.inputs.is_empty() {
            return Err("No input files or directories provided".to_string());
        }

        if let Some(output) = &self.output {
            if let Err(e) = std::fs::create_dir_all(output) {
                return Err(format!("Output directory is not writable: {}", e));
            }
        }

        if let Some(completed_dir) = &self.completed_directory {
            if let Err(e) = std::fs::create_dir_all(completed_dir) {
                return Err(format!("Completed directory is not writable: {}", e));
            }
        }

        Ok(())
    }
}
