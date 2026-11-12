use clap::{CommandFactory, FromArgMatches, Parser};
use std::path::PathBuf;
use tracing::{error, info, warn};

use m4b_merge::api::{MetadataRegion, MetadataSourceKind};
use m4b_merge::audio::FFmpeg;
use m4b_merge::config::Config;
use m4b_merge::processor::{ProcessingProgress, ProcessingStage, Processor, ProgressHandler};
/// A CLI tool which outputs consistently sorted, tagged, single m4b files
#[derive(Parser, Debug)]
#[command(name = "m4b-merge")]
#[command(author = "djdembeck")]
#[command(version = m4b_merge::VERSION)]
#[command(about = "A CLI tool which outputs consistently sorted, tagged, single m4b files", long_about = None)]
struct Args {
    /// Input files or directories to process (required)
    #[arg(short = 'i', long = "inputs", value_name = "PATH")]
    pub inputs: Vec<PathBuf>,

    /// Output directory for merged files
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Override the metadata source's default API URL (optional)
    #[arg(long = "api-url", value_name = "URL")]
    pub api_url: Option<String>,

    /// Metadata source to use for lookups
    #[arg(long = "metadata-source", value_enum, default_value_t = MetadataSourceKind::Audiobookdb)]
    pub metadata_source: MetadataSourceKind,

    /// Audible region for Audnexus metadata lookups
    #[arg(long = "region", value_enum, default_value_t = MetadataRegion::Us)]
    pub region: MetadataRegion,

    /// Directory path to move original input files to after processing
    #[arg(long = "completed-directory", value_name = "PATH")]
    pub completed_directory: Option<PathBuf>,

    /// Number of CPUs to use for parallel processing
    #[arg(long = "num-cpus", value_name = "N", default_value_t = { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) })]
    pub num_cpus: usize,

    /// Set logging level (error, warn, info, debug, trace)
    #[arg(long = "log-level", value_name = "LEVEL", default_value = "info")]
    pub log_level: String,

    /// Structure of output path/naming template
    #[arg(
        short = 'p',
        long = "path-format",
        value_name = "TEMPLATE",
        default_value = "{author}/{title}"
    )]
    pub path_format: String,

    /// Metadata ID (ASIN, AudiobookDB book ID, etc.) (optional)
    #[arg(short = 'a', long = "metadata-id", value_name = "ID")]
    pub metadata_id: Option<String>,

    /// Process a book from a versioned JSON job file. The job is the
    /// authoritative source for the metadata base (a provider id with an
    /// optional exact release, or fully supplied values) plus a manual
    /// overrides layer and the output root/template. Mutually exclusive with
    /// -i, -a, -o, -p, and --completed-directory.
    #[arg(long = "job", value_name = "FILE")]
    pub job: Option<PathBuf>,

    /// Show what would be done without actually doing it
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Non-mutating job preview: print the exact sanitized relative output
    /// path (plus effective values) as JSON, from the same resolver and
    /// metadata as execution. No directories, media, or name reservations.
    /// Job mode only; mutually exclusive with `--machine`.
    #[arg(long = "job-preview")]
    pub job_preview: bool,

    /// Machine-readable execution results: stable JSON on stdout only (logs
    /// and progress go to stderr), enumerating every produced output and
    /// sidecar, per-group outcome, and diagnosed errors. Job mode only;
    /// exits nonzero on any failure. Mutually exclusive with `--job-preview`.
    #[arg(long = "machine")]
    pub machine: bool,

    /// Check FFmpeg installation and display version information
    #[arg(long = "check-ffmpeg")]
    pub check_ffmpeg: bool,

    /// Print the machine-readable schema/resolver identity and supported
    /// token/field-policy capabilities as JSON, then exit.
    #[arg(long = "capabilities")]
    pub capabilities: bool,
}

/// Console progress handler that prints to stdout
struct ConsoleProgressHandler;

impl ProgressHandler for ConsoleProgressHandler {
    fn on_progress(&self, progress: ProcessingProgress) {
        write_progress(&progress, |s| println!("{s}"));
    }
}

/// Progress handler for `--machine` mode: every line goes to stderr so the
/// stdout stream stays a single clean JSON document.
struct StderrProgressHandler;

impl ProgressHandler for StderrProgressHandler {
    fn on_progress(&self, progress: ProcessingProgress) {
        write_progress(&progress, |s| eprintln!("{s}"));
    }
}

fn write_progress(progress: &ProcessingProgress, write: impl Fn(&str)) {
    match progress.stage {
        ProcessingStage::Complete => write(&format!("✓ {}", progress.message)),
        ProcessingStage::Error => write(&format!("✗ {}", progress.message)),
        _ => write(&format!(
            "[{}/{}] [{}] {}",
            progress.completed_files, progress.total_files, progress.stage, progress.message
        )),
    }
}

/// Print the machine result to stdout and exit: 0 when every group
/// succeeded, 1 otherwise. Never returns.
fn machine_exit(result: &m4b_merge::job::JobResult) -> ! {
    println!("{}", serde_json::to_string_pretty(result).expect("result serializes"));
    std::process::exit(if result.all_succeeded() { 0 } else { 1 });
}

/// Tracing writer: stderr in `--machine`/`--job-preview` mode (stdout must
/// stay a single clean JSON document), stdout elsewhere.
#[derive(Clone)]
struct LogWriter {
    stderr: bool,
}

impl tracing_subscriber::fmt::MakeWriter<'_> for LogWriter {
    type Writer = Box<dyn std::io::Write + Send>;
    fn make_writer(&self) -> Self::Writer {
        if self.stderr { Box::new(std::io::stderr()) } else { Box::new(std::io::stdout()) }
    }
}

#[tokio::main]
async fn main() {
    let matches = Args::command().try_get_matches().unwrap_or_else(|e| e.exit());
    let args = Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    // --capabilities is self-contained: no FFmpeg, no job, no network.
    if args.capabilities {
        println!("{}", m4b_merge::capabilities::Capabilities::build().to_json());
        std::process::exit(0);
    }

    // Initialize tracing with the requested log level. In `--machine` and
    // `--job-preview` mode the stdout stream is a single JSON document, so
    // every log line goes to stderr.
    let builder = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&args.log_level)),
    );
    let writer = LogWriter { stderr: args.machine || args.job_preview };
    let subscriber = builder.with_writer(writer).finish();

    if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("Warning: Failed to set tracing subscriber: {}", err);
        // Continue anyway - logging setup failure shouldn't prevent the app from running
    }

    // Handle --check-ffmpeg flag
    if args.check_ffmpeg {
        check_ffmpeg_and_exit();
    }

    // Job mode: the job file owns inputs, output root, and metadata base.
    // The standalone flags that would fight the job are rejected up front.
    let job = if let Some(job_path) = &args.job {
        let conflicts = job_flag_conflicts(&args, &matches);
        if !conflicts.is_empty() {
            eprintln!(
                "Error: --job is mutually exclusive with {} (the job file provides them)",
                conflicts.join(", ")
            );
            std::process::exit(1);
        }
        match m4b_merge::job::Job::load(job_path) {
            Ok(job) => Some(job),
            Err(e) => {
                // `--machine` promises a parseable document on every failure
                // path, including an unreadable or invalid job file.
                if args.machine {
                    machine_exit(&m4b_merge::job::JobResult::setup_failed(format!(
                        "failed to load job {}: {e}",
                        job_path.display()
                    )));
                }
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        if args.inputs.is_empty() {
            eprintln!("Error: No input files or directories provided.");
            eprintln!("Use -i or --inputs to specify input paths, or --job for a job file.");
            std::process::exit(1);
        }
        None
    };

    // In job mode the job file owns the input list; standalone mode keeps
    // the CLI flag inputs.
    let run_inputs: Vec<PathBuf> = match job.as_ref() {
        Some(job) => job.inputs.clone(),
        None => args.inputs.clone(),
    };

    // `--machine --dry-run` is rejected up front: the dry-run early exit
    // below would otherwise succeed without emitting any machine JSON.
    if args.machine && args.dry_run {
        eprintln!("Error: --machine is a real run; drop --dry-run.");
        std::process::exit(1);
    }

    // Early dry-run: print summary and exit without needing FFmpeg or output dir
    if args.dry_run {
        println!("=== Dry Run Mode ===");
        if let Some(job) = &job {
            println!("Job: {}", args.job.as_ref().expect("job mode").display());
            println!("Inputs:");
            for input in &job.inputs {
                println!("  - {}", input.display());
            }
            println!("Output Root: {}", job.output_root.display());
            println!(
                "Path Format: {}",
                if job.path_format.is_empty() {
                    "(default {author}/{title})"
                } else {
                    job.path_format.as_str()
                }
            );
        } else {
            println!("Inputs:");
            for input in &args.inputs {
                println!("  - {}", input.display());
            }
            if let Some(ref output) = args.output {
                println!("Output: {}", output.display());
            } else {
                println!("Output: (default)");
            }
            if let Some(ref completed_dir) = args.completed_directory {
                println!("Completed Directory: {}", completed_dir.display());
            }
            if let Some(metadata_id) = &args.metadata_id {
                println!("Metadata ID: {}", metadata_id);
            }
            println!("Metadata Source: {}", args.metadata_source);
        }
        println!("\nDry run complete. No files were modified.");
        std::process::exit(0);
    }

    // Create configuration. In job mode the CLI-level paths are unused; the
    // job's output_root, inputs, and completed_directory drive the run.
    let config = Config::new(
        run_inputs.clone(),
        args.output.clone(),
        args.api_url.clone(),
        args.metadata_source,
        args.region,
        args.completed_directory,
        args.num_cpus,
        args.log_level,
        args.path_format,
        args.dry_run,
        args.metadata_id,
    );

    // Validate configuration. The preview skips this deliberately:
    // `validate` creates the output and completed directories, and the
    // preview must mutate nothing (in job mode the CLI paths are unused
    // anyway; the job file is already validated at load).
    if !args.job_preview {
        if let Err(e) = config.validate() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    // `--api-url` no longer implies the audnexus source; with the default
    // audiobookdb source it points the AudiobookDB client at a custom URL, which
    // usually means an Audnexus proxy was intended for the audnexus source.
    if args.api_url.is_some() && args.metadata_source == MetadataSourceKind::Audiobookdb {
        warn!(
            "--api-url is being passed to the AudiobookDB client; use --metadata-source audnexus if it points to an Audnexus proxy"
        );
    }

    if args.region != MetadataRegion::Us && args.metadata_source == MetadataSourceKind::Audiobookdb
    {
        warn!(
            "--region only applies to the audnexus metadata source; it is ignored for AudiobookDB lookups"
        );
    }

    // --job-preview and --machine are job-mode-only surfaces; both need the
    // job and are mutually exclusive with each other.
    if args.job_preview || args.machine {
        let Some(job) = job.as_ref() else {
            eprintln!("Error: --job-preview/--machine require --job.");
            std::process::exit(1);
        };
        if args.job_preview && args.machine {
            eprintln!("Error: --job-preview and --machine are mutually exclusive.");
            std::process::exit(1);
        }
        if args.job_preview {
            // Non-mutating: resolves the effective metadata (a supplied job
            // needs no catalog) and the exact sanitized relative path via the
            // same resolver execution uses. No output/completed directories,
            // no media, no name reservation, no success promise.
            let preview = match job.preview(&config).await {
                Ok(preview) => preview,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            println!("{}", serde_json::to_string_pretty(&preview).expect("preview serializes"));
            std::process::exit(0);
        }
        // --machine: stable JSON on stdout only; progress goes to stderr via
        // StderrProgressHandler. Every failure path — setup, the run itself,
        // and any group failure — prints a truthful document before the
        // nonzero exit.
        let processor = match Processor::for_job(job.clone(), config) {
            Ok(processor) => processor,
            Err(e) => {
                error!("Failed to create processor: {}", e);
                machine_exit(&m4b_merge::job::JobResult::setup_failed(e.to_string()));
            }
        };
        let run = match processor.run(job.inputs.clone(), &StderrProgressHandler).await {
            Ok(run) => run,
            Err(e) => {
                error!("Processing failed: {}", e);
                machine_exit(&m4b_merge::job::JobResult::setup_failed(e.to_string()));
            }
        };
        machine_exit(&m4b_merge::job::JobResult::from_run(job.schema_version, &run));
    }

    info!("m4b-merge starting with {} input(s)", run_inputs.len());

    // Create and run processor. Job mode builds the processor from the job
    // file (its source selection and output root); standalone mode keeps the
    // legacy flag-driven behavior.
    let processor_result = match job {
        Some(job) => Processor::for_job(job, config),
        None => Processor::new(config),
    };

    match processor_result {
        Ok(processor) => {
            let progress_handler = ConsoleProgressHandler;

            match processor.process_with_progress(run_inputs, &progress_handler).await {
                Ok(results) => {
                    println!("\n=== Processing Complete ===");
                    println!("Successfully processed {} audiobook(s)", results.len());

                    for (idx, result) in results.iter().enumerate() {
                        println!("\n{}. {}", idx + 1, result.output_file.display());
                        println!("   Input files: {}", result.input_files.len());
                        println!(
                            "   Metadata applied: {}",
                            if result.metadata_applied { "Yes" } else { "No" }
                        );
                        println!(
                            "   Source files moved: {}",
                            if result.files_moved { "Yes" } else { "No" }
                        );
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

/// Standalone flags that a `--job` run must not also carry. An explicit
/// `-p/--path-format` is detected by clap's value source, not by comparing
/// against the default: writing the default template out verbatim must still
/// conflict with the job's own `path_format`.
fn job_flag_conflicts(args: &Args, matches: &clap::ArgMatches) -> Vec<&'static str> {
    // A default-filled value reports `ValueSource::DefaultValue`; only an
    // explicitly supplied value reports `CommandLine` (or Environment).
    let path_format_explicit = matches!(
        matches.value_source("path_format"),
        Some(clap::parser::ValueSource::CommandLine | clap::parser::ValueSource::EnvVariable)
    );
    [
        (!args.inputs.is_empty()).then_some("-i/--inputs"),
        args.metadata_id.is_some().then_some("-a/--metadata-id"),
        args.output.is_some().then_some("-o/--output"),
        path_format_explicit.then_some("-p/--path-format"),
        args.completed_directory.is_some().then_some("--completed-directory"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn check_ffmpeg_and_exit() -> ! {
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
                    std::process::exit(0);
                }
                Err(e) => {
                    error!("Failed to get FFmpeg version: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            error!("FFmpeg not found: {}", e);
            eprintln!(
                "Error: FFmpeg not found. Please install FFmpeg and ensure it's in your PATH."
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> (Args, clap::ArgMatches) {
        let matches = Args::command().try_get_matches_from(argv).unwrap();
        let args = Args::from_arg_matches(&matches).unwrap();
        (args, matches)
    }

    #[test]
    fn explicit_default_path_format_conflicts_with_job() {
        // Writing out the default template verbatim must still conflict with
        // the job's own path_format: value_source, not value comparison.
        let (args, matches) = parse(&["m4b-merge", "--job", "job.json", "-p", "{author}/{title}"]);
        let conflicts = job_flag_conflicts(&args, &matches);
        assert!(conflicts.contains(&"-p/--path-format"), "{conflicts:?}");
    }

    #[test]
    fn absent_path_format_does_not_conflict_with_job() {
        let (args, matches) = parse(&["m4b-merge", "--job", "job.json"]);
        let conflicts = job_flag_conflicts(&args, &matches);
        assert!(!conflicts.contains(&"-p/--path-format"), "{conflicts:?}");
    }

    #[test]
    fn explicit_custom_path_format_conflicts_with_job() {
        let (args, matches) = parse(&["m4b-merge", "--job", "job.json", "-p", "custom"]);
        let conflicts = job_flag_conflicts(&args, &matches);
        assert!(conflicts.contains(&"-p/--path-format"), "{conflicts:?}");
    }
}
