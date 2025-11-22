pub mod config;
pub mod job;
pub mod scan;
pub mod ffprobe;
pub mod classifier;
pub mod ffmpeg_native;
pub mod sidecar;
pub mod quality;
pub mod test_clip;

// Re-export commonly used types and functions
pub use config::TranscodeConfig;
pub use job::{Job, JobStatus};
pub use ffprobe::{FFProbeData, FFProbeFormat, FFProbeStream, BitDepth};
pub use classifier::WebSourceDecision;
pub use ffmpeg_native::{FFmpegManager, AV1Encoder, FFmpegVersion, CommandBuilder, ValidationResult, FFmpegResult};
pub use quality::{QualityCalculator, EncodingParams};
pub use test_clip::{TestClipWorkflow, TestClipInfo, ApprovalDecision};

