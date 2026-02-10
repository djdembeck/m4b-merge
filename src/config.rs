use std::path::PathBuf;

/// Configuration struct that holds all settings for m4b-merge
#[derive(Debug, Clone)]
#[allow(dead_code)]
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

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ConfigError {
    #[error("No input files or directories provided")]
    NoInputs,

    #[error("Input path does not exist: {0}")]
    InvalidInput(PathBuf),

    #[error("Output path is not a valid directory: {0}")]
    InvalidOutput(PathBuf),
}