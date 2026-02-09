use clap::Parser;
use std::path::PathBuf;
use tracing::error;

use m4b_merge::audio::FFmpeg;
use m4b_merge::config::Config;

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

    /// Show what would be done without actually doing it
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Check FFmpeg installation and display version information
    #[arg(long = "check-ffmpeg")]
    pub check_ffmpeg: bool,
}

fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    if args.check_ffmpeg {
        check_ffmpeg_and_exit();
    }

    let _config = Config::new(
        args.inputs,
        args.output,
        args.api_url,
        args.completed_directory,
        args.num_cpus,
        args.log_level,
        args.path_format,
        args.dry_run,
    );

    println!("m4b-merge configuration loaded");
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
            std::process::exit(1);
        }
    }

    std::process::exit(0);
}
