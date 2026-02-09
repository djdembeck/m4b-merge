pub mod ffmpeg;

pub use ffmpeg::{
    AudioMetadata, FFmpeg, FFmpegError, FFmpegVersion, LibraryVersion, Result, TimeRange,
};

pub struct AudioProcessor;

impl AudioProcessor {
    pub fn new() -> Self {
        Self {}
    }
}