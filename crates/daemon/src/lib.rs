pub mod config;
pub mod job;
pub mod scan;
pub mod ffprobe;
pub mod classifier;
pub mod ffmpeg_docker;
pub mod sidecar;

// Re-export commonly used functions
pub use ffmpeg_docker::calculate_optimal_quality;

pub use config::TranscodeConfig;
pub use job::{Job, JobStatus};
pub use ffprobe::{FFProbeData, FFProbeFormat, FFProbeStream};
pub use classifier::WebSourceDecision;

