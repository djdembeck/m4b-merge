pub mod ffmpeg;

pub use ffmpeg::{
    AudioMetadata, FFmpeg, FFmpegError, FFmpegVersion, LibraryVersion, Result, TimeRange,
};

/// Placeholder for future audio processing features.
///
/// TODO: Implement audio processing capabilities such as:
/// - Silence detection for chapter markers
/// - Audio normalization
/// - Volume normalization across tracks
/// - Sample rate conversion
#[allow(dead_code)]
#[derive(Default)]
pub struct AudioProcessor;

impl AudioProcessor {
    pub fn new() -> Self {
        Self
    }
}
