use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info};

use m4b_merge::audio::FFmpeg;
use m4b_merge::config::Config;
use m4b_merge::processor::{Processor, ProcessingProgress, ProcessingStage, ProgressHandler};

/// A CLI tool which outputs consistently sorted, tagged, single m4b files
#[derive(Parser, Debug)]
#[command(name = "m4b-merge")]
#[command(author = "djdembeck")]
#[command(version = "0.1.0")]
#[command(about = "A CLI tool which outputs consistently sorted, tagged, single m4b files", long_about = None)]
struct Args {
    /// Input files or directories to process (required)
    #[arg(short = 'i', long = "inputs", value_name = "PATH")]
    pub inputs: Vec<PathBuf>,

    /// Output directory for merged files
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Audnexus API URL to use for metadata lookup
    #[arg(long = "api_url", value_name = "URL", default_value = "https://api.audnex.us")]
    pub api_url: String,

    /// Directory path to move original input files to after processing
    #[arg(long = "completed_directory", value_name = "PATH")]
    pub completed_directory: Option<PathBuf>,

    /// Number of CPUs to use for parallel processing
    #[arg(long = "num_cpus", value_name = "N", default_value = "1")]
    pub num_cpus: usize,

    /// Set logging level (error, warn, info, debug, trace)
    #[arg(long = "log_level", value_name = "LEVEL", default_value = "info")]
    pub log_level: String,

    /// Structure of output path/naming template
    #[arg(short = 'p', long = "path_format", value_name = "TEMPLATE", default_value = "{author}/{title}")]
    pub path_format: String,

    /// ASIN for metadata lookup (optional)
    #[arg(short = 'a', long = "asin", value_name = "ASIN")]
    pub asin: Option<String>,

    /// Show what would be done without actually doing it
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Check FFmpeg installation and display version information
    #[arg(long = "check-ffmpeg")]
    pub check_ffmpeg: bool,
}

/// Console progress handler that prints to stdout
struct ConsoleProgressHandler;

impl ProgressHandler for ConsoleProgressHandler {
    fn on_progress(&self, progress: ProcessingProgress) {
        match progress.stage {
            ProcessingStage::Complete => {
                println!("✓ {}", progress.message);
            }
            ProcessingStage::Error => {
                eprintln!("✗ {}", progress.message);
            }
            _ => {
                println!("[{}/{}] [{}] {}",
                    progress.completed_files,
                    progress.total_files,
                    progress.stage,
                    progress.message
                );
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Initialize tracing with the requested log level
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new(&args.log_level)
                })
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");

    // Handle --check-ffmpeg flag
    if args.check_ffmpeg {
        check_ffmpeg_and_exit();
    }

    // Validate inputs
    if args.inputs.is_empty() {
        eprintln!("Error: No input files or directories provided.");
        eprintln!("Use -i or --inputs to specify input paths.");
        std::process::exit(1);
    }

    // Handle dry-run mode before consuming args
    if args.dry_run {
        println!("Dry run mode - showing what would be done:");
        println!("  Inputs: {:?}", args.inputs);
        println!("  Output: {:?}", args.output);
        println!("  Completed directory: {:?}", args.completed_directory);
        std::process::exit(0);
    }

    // Create configuration
    let config = Config::new(
        args.inputs.clone(),
        args.output.clone(),
        args.api_url,
        args.completed_directory,
        args.num_cpus,
        args.log_level,
        args.path_format,
        args.dry_run,
        args.asin,
    );

    info!("m4b-merge starting with {} input(s)", args.inputs.len());

    // Create and run processor
    match Processor::new(config) {
        Ok(processor) => {
            let progress_handler = ConsoleProgressHandler;

            match processor.process_with_progress(args.inputs, &progress_handler).await {
                Ok(results) => {
                    println!("\n=== Processing Complete ===");
                    println!("Successfully processed {} audiobook(s)", results.len());

                    for (idx, result) in results.iter().enumerate() {
                        println!("\n{}. {}", idx + 1, result.output_file.display());
                        println!("   Input files: {}", result.input_files.len());
                        println!("   Metadata applied: {}", if result.metadata_applied { "Yes" } else { "No" });
                        println!("   Source files moved: {}", if result.files_moved { "Yes" } else { "No" });
                    }

                    info!("m4b-merge completed successfully");
                }
                Err(e) => {
                    error!("Processing failed: {}", e);
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            error!("Failed to create processor: {}", e);
            eprintln!("Error: Failed to initialize: {}", e);
            std::process::exit(1);
        }
    }
}

fn check_ffmpeg_and_exit() {
    match FFmpeg::discover() {
        Ok(mut ffmpeg) => {
            println!("FFmpeg found:");
            println!("  Binary path: {}", ffmpeg.ffmpeg_path().display());
            println!("  FFprobe path: {}", ffmpeg.ffprobe_path().display());

            match ffmpeg.check() {
                Ok(()) => {
                    println!("  Status: OK");
                }
                Err(e) => {
                    error!("FFmpeg check failed: {}", e);
                    std::process::exit(1);
                }
            }

            match ffmpeg.version() {
                Ok(version) => {
                    println!("  Version: {}", version.version);
                    println!("  {}", version.copyright);
                    println!("  {}", version.built_with);
                    println!("\n  Libraries:");
                    for lib in &version.libraries {
                        println!(
                            "    {}: {} (compiled: {})",
                            lib.name, lib.current_version, lib.compiled_version
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to get FFmpeg version: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            error!("FFmpeg not found: {}", e);
            eprintln!("Error: FFmpeg not found. Please install FFmpeg and ensure it's in your PATH.");
            std::process::exit(1);
        }
    }

    std::process::exit(0);
}
