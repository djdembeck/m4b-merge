use std::path::PathBuf;

/// Configuration struct that holds all settings for m4b-merge
#[derive(Debug, Clone)]
pub struct Config {
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
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
            completed_directory,
            num_cpus,
            log_level,
            path_format,
            dry_run,
            asin,
        }
    }
}
