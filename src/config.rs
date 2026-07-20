use std::path::PathBuf;

/// Configuration struct that holds all settings for m4b-merge
#[derive(Debug, Clone)]
pub struct Config {
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub api_url: String,
    pub completed_directory: Option<PathBuf>,
    pub num_cpus: usize,
    pub log_level: String,
    pub path_format: String,
    pub dry_run: bool,
    pub asin: Option<String>,
}

impl Config {
    /// Create a new Config from CLI arguments
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        inputs: Vec<PathBuf>,
        output: Option<PathBuf>,
        api_url: String,
        completed_directory: Option<PathBuf>,
        num_cpus: usize,
        log_level: String,
        path_format: String,
        dry_run: bool,
        asin: Option<String>,
    ) -> Self {
        Self {
            inputs,
            output,
            api_url,
            completed_directory,
            num_cpus,
            log_level,
            path_format,
            dry_run,
            asin,
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
