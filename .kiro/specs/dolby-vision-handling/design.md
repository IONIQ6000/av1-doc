# Design Document

## Overview

This design document specifies the implementation of robust Dolby Vision (DV) metadata detection and removal for the AV1 encoding daemon. The system must automatically identify video files containing Dolby Vision metadata and strip it before encoding with Intel QSV, as QSV's AV1 encoder cannot properly handle DV metadata, resulting in corrupted output files.

The solution involves three main components:
1. **Detection Layer**: Multi-method DV detection in FFProbe metadata parsing
2. **Parameter Layer**: Encoding parameter structure that tracks DV presence
3. **Filter Layer**: FFmpeg filter chain that strips DV and converts to HDR10

## Architecture

### Component Diagram

```
┌─────────────────┐
│  Input Video    │
│  (with DV)      │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   FFProbe       │
│   Analysis      │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  DV Detection   │◄─── Multiple detection methods
│  (3 methods)    │     - Color transfer
└────────┬────────┘     - Stream tags
         │              - Codec name
         ▼
┌─────────────────┐
│  Encoding       │
│  Parameters     │◄─── has_dolby_vision flag
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Filter Chain   │
│  Construction   │
└────────┬────────┘
         │
         ├─── DV Present? ──► Yes ──► DV Stripping Filters
         │                            (zscale + tonemap)
         │
         └─── No ──────────► Standard Filters
                             (format + hwupload)
         │
         ▼
┌─────────────────┐
│  QSV Encoding   │
│  (AV1)          │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Output Video   │
│  (HDR10, no DV) │
└─────────────────┘
```

### Data Flow

1. **Input**: Video file with potential DV metadata
2. **Probe**: FFProbe extracts stream and format metadata
3. **Detect**: System checks for DV markers using multiple methods
4. **Flag**: Encoding parameters include DV detection result
5. **Filter**: If DV detected, apply stripping filter chain before encoding
6. **Encode**: QSV encodes clean HDR10 video without DV corruption
7. **Output**: Compatible AV1 file that transcodes correctly

## Components and Interfaces

### 1. FFProbe Detection Module

**Location**: `crates/daemon/src/ffprobe.rs`

**Structures**:

```rust
pub struct FFProbeStream {
    pub color_transfer: Option<String>,
    pub codec_name: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    // ... other fields
}

pub struct FFProbeData {
    pub streams: Vec<FFProbeStream>,
    pub format: FFProbeFormat,
}
```

**Methods**:

```rust
impl FFProbeStream {
    /// Check if this stream contains Dolby Vision metadata
    /// Uses three detection methods for robustness
    pub fn has_dolby_vision(&self) -> bool;
}

impl FFProbeData {
    /// Check if any video stream has Dolby Vision
    pub fn has_dolby_vision(&self) -> bool;
}
```

### 2. Encoding Parameters Module

**Location**: `crates/daemon/src/ffmpeg_docker.rs`

**Structure**:

```rust
pub struct EncodingParams {
    pub bit_depth: BitDepth,
    pub pixel_format: String,
    pub av1_profile: u8,
    pub qp: i32,
    pub is_hdr: bool,
    pub has_dolby_vision: bool,  // NEW FIELD
}
```

**Function**:

```rust
pub fn determine_encoding_params(
    meta: &FFProbeData,
    input_file: &Path,
) -> EncodingParams;
```

### 3. Filter Chain Construction Module

**Location**: `crates/daemon/src/ffmpeg_docker.rs`

**Function**:

```rust
pub async fn run_av1_qsv_job(
    cfg: &TranscodeConfig,
    input: &Path,
    temp_output: &Path,
    meta: &FFProbeData,
    decision: &WebSourceDecision,
    encoding_params: &EncodingParams,
) -> Result<FFmpegResult>;
```

**Filter Chain Logic**:
- Standard filters: `pad → setsar → format → hwupload`
- DV stripping filters: `pad → setsar → zscale(linear) → format(float) → zscale(bt709) → tonemap → zscale(bt709 TV) → format → hwupload`

## Data Models

### Dolby Vision Markers

The system detects DV through multiple indicators:

**Color Transfer Values**:
- `smpte2094` - SMPTE ST 2094 standard (Dolby Vision)
- `st2094` - Abbreviated form

**Codec Tags**:
- `dovi` - Dolby Vision codec identifier
- `dvcl` - DV compatibility layer
- `dvhe` - DV with HEVC
- `dvh1` - DV variant

**Stream Tag Keys/Values**:
- `dolby` - Generic Dolby marker
- `dovi` - Dolby Vision identifier

### Filter Chain Components

**DV Stripping Pipeline**:

1. **Linearization**: `zscale=t=linear:npl=100`
   - Converts to linear color space
   - Normalizes peak luminance to 100 nits

2. **Float Conversion**: `format=gbrpf32le`
   - High precision floating-point format
   - Preserves color accuracy during processing

3. **Primary Remapping**: `zscale=p=bt709`
   - Converts color primaries to BT.709

4. **Tonemapping**: `tonemap=tonemap=hable:desat=0`
   - Applies Hable (Uncharted 2) tonemapping
   - Zero desaturation preserves color vibrancy

5. **TV Range Conversion**: `zscale=t=bt709:m=bt709:r=tv`
   - Transfer: BT.709
   - Matrix: BT.709
   - Range: TV (limited range)

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*


### Property 1: Color Transfer Detection

*For any* FFProbe stream metadata with color transfer containing "smpte2094" or "st2094", the detection method SHALL return true for Dolby Vision presence
**Validates: Requirements 1.1**

### Property 2: Stream Tag Detection

*For any* FFProbe stream with tags containing "dolby", "dovi", "dvcl", "dvhe", or "dvh1" in keys or values, the detection method SHALL return true for Dolby Vision presence
**Validates: Requirements 1.2**

### Property 3: Codec Name Detection

*For any* FFProbe stream with codec name containing "dovi" or "dolby", the detection method SHALL return true for Dolby Vision presence
**Validates: Requirements 1.3**

### Property 4: Multi-Stream Detection

*For any* FFProbe data with multiple video streams, if any single video stream contains DV markers, the file-level detection SHALL return true
**Validates: Requirements 1.4**

### Property 5: Detection Method Independence

*For any* FFProbe stream with DV markers detectable by any single method (color transfer, tags, or codec name), the detection SHALL succeed regardless of which method finds it
**Validates: Requirements 1.5**

### Property 6: Encoding Parameters Include DV Flag

*For any* video file metadata, the encoding parameters determined from that metadata SHALL include a has_dolby_vision boolean field
**Validates: Requirements 2.1**

### Property 7: DV Filter Chain Construction

*For any* encoding job where encoding parameters indicate Dolby Vision presence, the constructed filter chain SHALL include DV stripping filters before standard format conversion
**Validates: Requirements 3.1**

### Property 8: Linearization Filter Presence

*For any* filter chain constructed for DV stripping, the filter string SHALL contain "zscale=t=linear:npl=100"
**Validates: Requirements 3.2**

### Property 9: Tonemapping Filter Presence

*For any* filter chain constructed for DV stripping, the filter string SHALL contain "tonemap=" filter
**Validates: Requirements 3.3**

### Property 10: BT.709 TV Range Conversion

*For any* filter chain constructed for DV stripping, the filter string SHALL contain "zscale=t=bt709:m=bt709:r=tv"
**Validates: Requirements 3.4**

### Property 11: Hable Tonemapping with Zero Desaturation

*For any* filter chain constructed for DV stripping, the tonemap filter SHALL specify "tonemap=hable:desat=0"
**Validates: Requirements 4.2**

### Property 12: Float Format Intermediate

*For any* filter chain constructed for DV stripping, the filter string SHALL contain "format=gbrpf32le" for high precision color processing
**Validates: Requirements 4.3**

### Property 13: Filter Chain Ordering

*For any* filter chain constructed for DV stripping, the DV stripping filters SHALL appear before the standard format conversion and hwupload filters
**Validates: Requirements 4.4**

### Property 14: No False Positives

*For any* FFProbe stream metadata without SMPTE ST 2094 color transfer, without DV-related tags, and without DV-related codec names, the detection method SHALL return false
**Validates: Requirements 6.4**

## Error Handling

### Detection Errors

**Missing Metadata**:
- If color_transfer field is None, skip that detection method
- If tags field is None, skip tag-based detection
- If codec_name is None, skip codec detection
- Detection succeeds if ANY method finds DV markers

**Malformed Data**:
- Use case-insensitive string matching to handle variations
- Handle both full and abbreviated forms (e.g., "smpte2094" and "st2094")
- Gracefully handle empty strings and whitespace

### Filter Construction Errors

**Invalid Parameters**:
- If encoding_params.has_dolby_vision is false, skip DV stripping filters entirely
- Ensure filter chain is always valid FFmpeg syntax
- Log filter chain construction for debugging

**Resource Constraints**:
- DV stripping adds computational overhead (tonemapping, color space conversion)
- Monitor encoding performance and log warnings if significantly slower
- Consider making DV stripping optional via configuration in future

### Encoding Errors

**QSV Compatibility**:
- If DV stripping fails, encoding will likely produce corrupted output
- Validate output file after encoding to detect corruption
- Log detailed error messages with filter chain details

**Fallback Strategy**:
- If DV detection is uncertain, prefer false positive (strip when not needed) over false negative (miss DV and corrupt output)
- Stripping non-DV HDR10 content is safe and produces valid output

## Testing Strategy

### Unit Testing

**Detection Logic Tests**:
- Test each detection method independently with known DV markers
- Test negative cases (no DV markers) to ensure no false positives
- Test edge cases: empty strings, None values, case variations
- Test multiple video streams with DV in different positions

**Parameter Construction Tests**:
- Verify has_dolby_vision flag is set correctly based on detection
- Test with various combinations of bit depth, HDR, and DV
- Ensure backward compatibility with existing encoding parameters

**Filter Chain Tests**:
- Verify DV stripping filters are added when flag is true
- Verify standard filters are used when flag is false
- Test filter chain ordering and syntax validity
- Verify all required filter components are present

### Property-Based Testing

The system uses the `proptest` crate for property-based testing. Each correctness property will be implemented as a property test that runs 100 iterations with randomly generated inputs.

**Property Test Structure**:
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    /// **Feature: dolby-vision-handling, Property N: [Property Name]**
    /// **Validates: Requirements X.Y**
    #[test]
    fn test_property_name(
        // Random input generators
    ) {
        // Test logic
        prop_assert!(condition, "error message");
    }
}
```

**Test Generators**:
- Random color transfer strings (with and without DV markers)
- Random tag dictionaries with various DV-related keys/values
- Random codec names
- Random combinations of detection methods
- Random encoding parameters with DV flag variations

**Property Test Coverage**:
- Property 1-5: Detection logic across all methods
- Property 6: Encoding parameter structure
- Property 7-13: Filter chain construction and ordering
- Property 14: False positive prevention

### Integration Testing

**End-to-End Tests**:
- Test with real video files containing Dolby Vision
- Verify output files are valid and not corrupted
- Test transcoding of output files in Plex
- Compare visual quality of DV-stripped vs original

**Performance Tests**:
- Measure encoding time overhead of DV stripping
- Test with various resolutions (1080p, 4K)
- Monitor CPU and GPU usage during DV stripping

**Compatibility Tests**:
- Test with different DV profiles (Profile 5, Profile 7, Profile 8)
- Test with different source codecs (HEVC, H.264)
- Test with different HDR formats (HDR10, HDR10+, HLG)

## Implementation Notes

### Detection Method Priority

All three detection methods are equally important:
1. **Color Transfer**: Most reliable for standards-compliant files
2. **Stream Tags**: Catches files with explicit DV tagging
3. **Codec Name**: Identifies DV-specific codecs

The OR logic ensures maximum detection coverage.

### Filter Chain Performance

DV stripping adds processing overhead:
- **Linearization**: Moderate CPU cost
- **Float Conversion**: Memory bandwidth intensive
- **Tonemapping**: Computationally expensive
- **Color Space Conversion**: Moderate CPU cost

Total overhead: ~15-25% longer encoding time compared to non-DV files.

### Quality Preservation

The Hable tonemapping algorithm preserves HDR appearance:
- Maintains highlight detail
- Preserves shadow information
- Zero desaturation keeps colors vibrant
- BT.709 TV range ensures compatibility

Output quality is visually similar to original DV, just without the proprietary enhancement layer.

### Future Enhancements

**Configuration Options**:
- Allow users to disable DV stripping (for advanced use cases)
- Support different tonemapping algorithms (Reinhard, Mobius, etc.)
- Configurable peak luminance normalization

**Advanced Detection**:
- Parse Dolby Vision RPU (Reference Processing Unit) data
- Detect DV profile and version
- Warn about specific DV profiles that may need special handling

**Output Validation**:
- Verify DV metadata is completely removed
- Check output file for any remaining DV side data
- Validate HDR10 metadata is preserved correctly
