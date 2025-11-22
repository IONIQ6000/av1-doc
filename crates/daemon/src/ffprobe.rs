use std::path::Path;
use std::collections::HashMap;
use anyhow::{Context, Result};
use serde::Deserialize;
use crate::config::TranscodeConfig;
use tokio::process::Command;

/// Complete ffprobe output structure
#[derive(Debug, Clone, Deserialize)]
pub struct FFProbeData {
    pub streams: Vec<FFProbeStream>,
    pub format: FFProbeFormat,
}

/// Format-level metadata from ffprobe
#[derive(Debug, Clone, Deserialize)]
pub struct FFProbeFormat {
    #[serde(rename = "format_name")]
    pub format_name: String,
    #[serde(rename = "bit_rate")]
    pub bit_rate: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    #[serde(rename = "muxing_app")]
    pub muxing_app: Option<String>,
    #[serde(rename = "writing_library")]
    pub writing_library: Option<String>,
}

/// Stream-level metadata from ffprobe
#[derive(Debug, Clone, Deserialize)]
pub struct FFProbeStream {
    pub index: i32,
    #[serde(rename = "codec_type")]
    pub codec_type: Option<String>,
    #[serde(rename = "codec_name")]
    pub codec_name: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "avg_frame_rate")]
    pub avg_frame_rate: Option<String>,
    #[serde(rename = "r_frame_rate")]
    pub r_frame_rate: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    #[serde(rename = "bit_rate")]
    pub bit_rate: Option<String>,
    pub disposition: Option<HashMap<String, i32>>,
    #[serde(rename = "pix_fmt")]
    pub pix_fmt: Option<String>,
    #[serde(rename = "bits_per_raw_sample")]
    pub bits_per_raw_sample: Option<String>,
    #[serde(rename = "color_transfer")]
    pub color_transfer: Option<String>,
    #[serde(rename = "color_primaries")]
    pub color_primaries: Option<String>,
    #[serde(rename = "color_space")]
    pub color_space: Option<String>,
}

/// Run ffprobe via Docker and parse the JSON output
pub async fn probe_file(cfg: &TranscodeConfig, file_path: &Path) -> Result<FFProbeData> {
    use log::debug;
    
    // Verify file exists before trying to probe
    if !file_path.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }
    
    // Get parent directory and basename for Docker volume mounting
    let parent_dir = file_path
        .parent()
        .context("File path has no parent directory")?;
    let basename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("File path has no basename")?;

    // Verify parent directory exists
    if !parent_dir.exists() {
        anyhow::bail!("Parent directory does not exist: {}", parent_dir.display());
    }

    // Container path will be /config/<basename>
    // Use proper escaping for paths with spaces/special chars
    let container_path = format!("/config/{}", basename);

    debug!("ffprobe: mounting {} to /config", parent_dir.display());
    debug!("ffprobe: probing file {} in container", container_path);

    // Build docker command
    // Note: Using --privileged flag required when Docker runs inside LXC containers
    // Use --entrypoint to bypass any entrypoint scripts that might interfere
    let mut cmd = Command::new(&cfg.docker_bin);
    cmd.arg("run")
        .arg("--rm")
        .arg("--privileged")
        .arg("--entrypoint")
        .arg("ffprobe")
        .arg("-v")
        .arg("/dev/dri:/dev/dri")
        .arg("-v")
        .arg(format!("{}:/config:ro", parent_dir.display()))
        .arg(&cfg.docker_image)
        .arg("-v")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(&container_path);
    
    debug!("ffprobe command: docker run --rm --privileged --entrypoint ffprobe --device {}:{} -v {}:/config:ro {} -v error -print_format json -show_streams -show_format {}",
           cfg.gpu_device.display(), cfg.gpu_device.display(), parent_dir.display(), cfg.docker_image, container_path);

    // Execute and capture output
    let output = cmd
        .output()
        .await
        .with_context(|| format!("Failed to execute docker ffprobe for: {}", file_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let exit_code = output.status.code().unwrap_or(-1);
        
        // Log the full command for debugging
        debug!("ffprobe command failed. Full command would be:");
        debug!("  docker run --rm --privileged --device {}:{} -v {}:/config:ro {} ffprobe ...",
               cfg.gpu_device.display(), cfg.gpu_device.display(),
               parent_dir.display(), cfg.docker_image);
        
        anyhow::bail!(
            "ffprobe failed (exit code {}) for {}:\nParent dir: {}\nBasename: {}\nContainer path: {}\nSTDERR: {}\nSTDOUT: {}",
            exit_code,
            file_path.display(),
            parent_dir.display(),
            basename,
            container_path,
            stderr,
            stdout
        );
    }

    // Parse JSON output
    let json_str = String::from_utf8(output.stdout)
        .context("ffprobe output is not valid UTF-8")?;

    let data: FFProbeData = serde_json::from_str(&json_str)
        .with_context(|| format!("Failed to parse ffprobe JSON for: {}", file_path.display()))?;

    Ok(data)
}


/// Bit depth of video content
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Bit8,
    Bit10,
    Unknown,
}

impl FFProbeStream {
    /// Detect bit depth from stream metadata
    /// Checks multiple sources: bits_per_raw_sample, pix_fmt, and HDR metadata
    pub fn detect_bit_depth(&self) -> BitDepth {
        // Method 1: Check bits_per_raw_sample (most reliable)
        if let Some(ref bits) = self.bits_per_raw_sample {
            if bits == "10" {
                return BitDepth::Bit10;
            } else if bits == "8" {
                return BitDepth::Bit8;
            }
        }
        
        // Method 2: Parse pixel format for "10" suffix
        if let Some(ref pix_fmt) = self.pix_fmt {
            let fmt_lower = pix_fmt.to_lowercase();
            if fmt_lower.contains("10") || fmt_lower.contains("p010") {
                return BitDepth::Bit10;
            }
        }
        
        // Method 3: Check HDR metadata (implies 10-bit)
        if self.is_hdr_content() {
            return BitDepth::Bit10;
        }
        
        // Default to 8-bit if unknown
        BitDepth::Bit8
    }
    
    /// Check if content is HDR (High Dynamic Range)
    /// HDR content requires 10-bit encoding
    pub fn is_hdr_content(&self) -> bool {
        // Check color transfer characteristics
        if let Some(ref transfer) = self.color_transfer {
            let t = transfer.to_lowercase();
            // PQ (Perceptual Quantizer) - HDR10
            if t.contains("smpte2084") || t.contains("st2084") {
                return true;
            }
            // HLG (Hybrid Log-Gamma) - HDR broadcast
            if t.contains("arib-std-b67") || t.contains("hlg") {
                return true;
            }
        }
        
        // Check color primaries (bt2020 often indicates HDR)
        if let Some(ref primaries) = self.color_primaries {
            let p = primaries.to_lowercase();
            if p.contains("bt2020") {
                // bt2020 with 10-bit is likely HDR
                if let Some(ref bits) = self.bits_per_raw_sample {
                    if bits == "10" {
                        return true;
                    }
                }
            }
        }
        
        false
    }
    
    /// Check if content has Dolby Vision metadata
    /// Dolby Vision can cause corruption with QSV AV1 encoding and should be stripped
    pub fn has_dolby_vision(&self) -> bool {
        // Method 1: Check color transfer for Dolby Vision
        if let Some(ref transfer) = self.color_transfer {
            let t = transfer.to_lowercase();
            // SMPTE ST 2094 is Dolby Vision
            if t.contains("smpte2094") || t.contains("st2094") {
                return true;
            }
        }
        
        // Method 2: Check stream tags for Dolby Vision markers
        if let Some(ref tags) = self.tags {
            for (key, value) in tags {
                let k = key.to_lowercase();
                let v = value.to_lowercase();
                
                // Check for DV in various tag fields
                if k.contains("dolby") || v.contains("dolby") {
                    return true;
                }
                if k.contains("dovi") || v.contains("dovi") {
                    return true;
                }
                // Check for DVCL/DVHE codec tags
                if v.contains("dvcl") || v.contains("dvhe") || v.contains("dvh1") {
                    return true;
                }
            }
        }
        
        // Method 3: Check codec name for Dolby Vision
        if let Some(ref codec) = self.codec_name {
            let c = codec.to_lowercase();
            if c.contains("dovi") || c.contains("dolby") {
                return true;
            }
        }
        
        false
    }
}

impl FFProbeData {
    /// Check if any video stream has Dolby Vision
    pub fn has_dolby_vision(&self) -> bool {
        self.streams.iter()
            .filter(|s| s.codec_type.as_deref() == Some("video"))
            .any(|s| s.has_dolby_vision())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Strategy to generate color transfer strings with DV markers
    fn color_transfer_with_dv() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("smpte2094".to_string()),
            Just("st2094".to_string()),
            Just("SMPTE2094".to_string()),
            Just("ST2094".to_string()),
            Just("smpte2094-40".to_string()),
        ]
    }

    // Strategy to generate color transfer strings without DV markers
    fn color_transfer_without_dv() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("smpte2084".to_string()),
            Just("bt709".to_string()),
            Just("bt2020".to_string()),
            Just("arib-std-b67".to_string()),
            Just("linear".to_string()),
        ]
    }

    // Strategy to generate tags with DV markers
    fn tags_with_dv() -> impl Strategy<Value = HashMap<String, String>> {
        prop_oneof![
            Just({
                let mut map = HashMap::new();
                map.insert("ENCODER".to_string(), "dolby vision".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("dovi_config".to_string(), "profile5".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("codec_tag".to_string(), "dvcl".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("codec_tag".to_string(), "dvhe".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("codec_tag".to_string(), "dvh1".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("DOLBY".to_string(), "true".to_string());
                map
            }),
        ]
    }

    // Strategy to generate tags without DV markers
    fn tags_without_dv() -> impl Strategy<Value = HashMap<String, String>> {
        prop_oneof![
            Just({
                let mut map = HashMap::new();
                map.insert("ENCODER".to_string(), "x265".to_string());
                map
            }),
            Just({
                let mut map = HashMap::new();
                map.insert("title".to_string(), "Movie".to_string());
                map
            }),
            Just(HashMap::new()),
        ]
    }

    // Strategy to generate codec names with DV markers
    fn codec_name_with_dv() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("dovi".to_string()),
            Just("dolby_vision".to_string()),
            Just("hevc_dovi".to_string()),
            Just("DOVI".to_string()),
        ]
    }

    // Strategy to generate codec names without DV markers
    fn codec_name_without_dv() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("hevc".to_string()),
            Just("h264".to_string()),
            Just("av1".to_string()),
            Just("vp9".to_string()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: dolby-vision-handling, Property 1: Color Transfer Detection**
        /// **Validates: Requirements 1.1**
        #[test]
        fn test_color_transfer_detection(
            color_transfer in color_transfer_with_dv(),
        ) {
            let stream = FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name: Some("hevc".to_string()),
                width: Some(1920),
                height: Some(1080),
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                tags: None,
                bit_rate: None,
                disposition: None,
                pix_fmt: None,
                bits_per_raw_sample: None,
                color_transfer: Some(color_transfer.clone()),
                color_primaries: None,
                color_space: None,
            };

            prop_assert!(
                stream.has_dolby_vision(),
                "Stream with color_transfer '{}' should be detected as Dolby Vision",
                color_transfer
            );
        }

        /// **Feature: dolby-vision-handling, Property 2: Stream Tag Detection**
        /// **Validates: Requirements 1.2**
        #[test]
        fn test_stream_tag_detection(
            tags in tags_with_dv(),
        ) {
            let stream = FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name: Some("hevc".to_string()),
                width: Some(1920),
                height: Some(1080),
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                tags: Some(tags.clone()),
                bit_rate: None,
                disposition: None,
                pix_fmt: None,
                bits_per_raw_sample: None,
                color_transfer: None,
                color_primaries: None,
                color_space: None,
            };

            prop_assert!(
                stream.has_dolby_vision(),
                "Stream with tags {:?} should be detected as Dolby Vision",
                tags
            );
        }

        /// **Feature: dolby-vision-handling, Property 3: Codec Name Detection**
        /// **Validates: Requirements 1.3**
        #[test]
        fn test_codec_name_detection(
            codec_name in codec_name_with_dv(),
        ) {
            let stream = FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name: Some(codec_name.clone()),
                width: Some(1920),
                height: Some(1080),
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                tags: None,
                bit_rate: None,
                disposition: None,
                pix_fmt: None,
                bits_per_raw_sample: None,
                color_transfer: None,
                color_primaries: None,
                color_space: None,
            };

            prop_assert!(
                stream.has_dolby_vision(),
                "Stream with codec_name '{}' should be detected as Dolby Vision",
                codec_name
            );
        }

        /// **Feature: dolby-vision-handling, Property 4: Multi-Stream Detection**
        /// **Validates: Requirements 1.4**
        #[test]
        fn test_multi_stream_detection(
            dv_stream_index in 0usize..3,
            color_transfer in color_transfer_with_dv(),
        ) {
            // Create multiple video streams, only one with DV
            let mut streams = vec![];
            
            for i in 0..3 {
                let stream = FFProbeStream {
                    index: i as i32,
                    codec_type: Some("video".to_string()),
                    codec_name: Some("hevc".to_string()),
                    width: Some(1920),
                    height: Some(1080),
                    avg_frame_rate: Some("24/1".to_string()),
                    r_frame_rate: Some("24/1".to_string()),
                    tags: None,
                    bit_rate: None,
                    disposition: None,
                    pix_fmt: None,
                    bits_per_raw_sample: None,
                    color_transfer: if i == dv_stream_index {
                        Some(color_transfer.clone())
                    } else {
                        Some("bt709".to_string())
                    },
                    color_primaries: None,
                    color_space: None,
                };
                streams.push(stream);
            }

            let data = FFProbeData {
                streams,
                format: FFProbeFormat {
                    format_name: "matroska".to_string(),
                    bit_rate: None,
                    tags: None,
                    muxing_app: None,
                    writing_library: None,
                },
            };

            prop_assert!(
                data.has_dolby_vision(),
                "FFProbeData with DV in stream {} should be detected as Dolby Vision",
                dv_stream_index
            );
        }

        /// **Feature: dolby-vision-handling, Property 5: Detection Method Independence**
        /// **Validates: Requirements 1.5**
        #[test]
        fn test_detection_method_independence(
            detection_method in 0u8..3,
        ) {
            let (color_transfer, tags, codec_name) = match detection_method {
                0 => (Some("smpte2094".to_string()), None, Some("hevc".to_string())),
                1 => (None, Some({
                    let mut map = HashMap::new();
                    map.insert("dovi".to_string(), "true".to_string());
                    map
                }), Some("hevc".to_string())),
                _ => (None, None, Some("dovi".to_string())),
            };

            let stream = FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name,
                width: Some(1920),
                height: Some(1080),
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                tags,
                bit_rate: None,
                disposition: None,
                pix_fmt: None,
                bits_per_raw_sample: None,
                color_transfer,
                color_primaries: None,
                color_space: None,
            };

            prop_assert!(
                stream.has_dolby_vision(),
                "Stream with detection method {} should be detected as Dolby Vision",
                detection_method
            );
        }

        /// **Feature: dolby-vision-handling, Property 14: No False Positives**
        /// **Validates: Requirements 6.4**
        #[test]
        fn test_no_false_positives(
            color_transfer in color_transfer_without_dv(),
            tags in tags_without_dv(),
            codec_name in codec_name_without_dv(),
        ) {
            let stream = FFProbeStream {
                index: 0,
                codec_type: Some("video".to_string()),
                codec_name: Some(codec_name.clone()),
                width: Some(1920),
                height: Some(1080),
                avg_frame_rate: Some("24/1".to_string()),
                r_frame_rate: Some("24/1".to_string()),
                tags: Some(tags.clone()),
                bit_rate: None,
                disposition: None,
                pix_fmt: None,
                bits_per_raw_sample: None,
                color_transfer: Some(color_transfer.clone()),
                color_primaries: None,
                color_space: None,
            };

            prop_assert!(
                !stream.has_dolby_vision(),
                "Stream without DV markers (color_transfer: '{}', codec_name: '{}', tags: {:?}) should NOT be detected as Dolby Vision",
                color_transfer, codec_name, tags
            );
        }
    }
}
