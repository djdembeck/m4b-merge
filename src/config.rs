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

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Check that inputs are not empty
        if self.inputs.is_empty() {
            return Err(ConfigError::NoInputs);
        }

        // Check that each input path exists
        for input in &self.inputs {
            if !input.exists() {
                return Err(ConfigError::InvalidInput(input.clone()));
            }
        }

        // Check that output directory is valid if specified
        if let Some(ref output) = self.output {
            if !output.is_dir() {
                return Err(ConfigError::InvalidOutput(output.clone()));
            }
        }

        // Check that completed_directory is valid if specified
        if let Some(ref completed_dir) = self.completed_directory {
            if !completed_dir.is_dir() {
                return Err(ConfigError::InvalidCompletedDirectory(completed_dir.clone()));
            }
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("No input files or directories provided")]
    NoInputs,

    #[error("Input path does not exist: {0}")]
    InvalidInput(PathBuf),

    #[error("Output path is not a valid directory: {0}")]
    InvalidOutput(PathBuf),

    #[error("Completed directory path is not a valid directory: {0}")]
    InvalidCompletedDirectory(PathBuf),
}
