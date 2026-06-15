pub mod ffmpeg;

pub use ffmpeg::{
    AudioMetadata, FFmpeg, FFmpegError, FFmpegVersion, LibraryVersion, MetadataProvider, Result,
    TimeRange,
};
