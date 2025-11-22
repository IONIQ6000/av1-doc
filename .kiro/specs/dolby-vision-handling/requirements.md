# Requirements Document

## Introduction

This specification defines the requirements for robust Dolby Vision (DV) metadata detection and removal in the AV1 encoding daemon. Intel QSV's AV1 encoder cannot properly handle Dolby Vision metadata, resulting in corrupted output files that play natively but fail when transcoded by strict decoders like Plex. The system must automatically detect DV content and strip the metadata before encoding, converting it to standard HDR10 which QSV handles correctly.

## Glossary

- **Dolby Vision (DV)**: A proprietary HDR format that adds an enhancement layer on top of HDR10 base layer
- **System**: The AV1 encoding daemon
- **FFProbe**: FFmpeg's media analysis tool used to extract metadata from video files
- **QSV**: Intel Quick Sync Video hardware encoder
- **HDR10**: Standard high dynamic range format with 10-bit color depth
- **SMPTE ST 2094**: Society of Motion Picture and Television Engineers standard for dynamic metadata (used by Dolby Vision)
- **Color Transfer**: Video metadata field describing the transfer characteristics (gamma curve)
- **Codec Tag**: Four-character code identifying video codec type
- **Stream Tags**: Key-value metadata pairs associated with video streams
- **Filter Chain**: Sequence of FFmpeg video processing filters applied during encoding
- **Tonemapping**: Process of converting high dynamic range content to different color spaces

## Requirements

### Requirement 1

**User Story:** As a system operator, I want the system to automatically detect Dolby Vision content, so that I can prevent encoding corruption without manual intervention.

#### Acceptance Criteria

1. WHEN the system probes a video file with SMPTE ST 2094 color transfer, THEN the system SHALL identify the file as containing Dolby Vision metadata
2. WHEN the system examines stream tags containing "dolby", "dovi", "dvcl", "dvhe", or "dvh1", THEN the system SHALL identify the file as containing Dolby Vision metadata
3. WHEN the system examines a codec name containing "dovi" or "dolby", THEN the system SHALL identify the file as containing Dolby Vision metadata
4. WHEN the system checks any video stream in a file, THEN the system SHALL report Dolby Vision presence if any video stream contains DV markers
5. WHEN multiple detection methods identify Dolby Vision, THEN the system SHALL report detection through any single method as sufficient

### Requirement 2

**User Story:** As a system operator, I want encoding parameters to include Dolby Vision status, so that the encoding process can adapt its behavior accordingly.

#### Acceptance Criteria

1. WHEN the system determines encoding parameters for a video file, THEN the system SHALL include a boolean flag indicating Dolby Vision presence
2. WHEN Dolby Vision is detected during parameter determination, THEN the system SHALL log a warning message indicating DV will be stripped
3. WHEN encoding parameters are displayed in logs, THEN the system SHALL include the Dolby Vision status alongside other parameters like bit depth and HDR status

### Requirement 3

**User Story:** As a system operator, I want Dolby Vision metadata automatically stripped during encoding, so that output files are compatible with all transcoders and players.

#### Acceptance Criteria

1. WHEN the system encodes a file with Dolby Vision metadata, THEN the system SHALL apply a filter chain that removes DV metadata before QSV encoding
2. WHEN stripping Dolby Vision, THEN the system SHALL convert the video to linear color space with normalized peak luminance
3. WHEN stripping Dolby Vision, THEN the system SHALL apply tonemapping to preserve HDR appearance
4. WHEN stripping Dolby Vision, THEN the system SHALL convert the result back to BT.709 color space with TV range
5. WHEN Dolby Vision stripping is applied, THEN the system SHALL log an informational message indicating the stripping operation

### Requirement 4

**User Story:** As a system operator, I want stripped files to maintain HDR10 quality, so that visual quality is preserved despite removing Dolby Vision enhancements.

#### Acceptance Criteria

1. WHEN the system strips Dolby Vision from a 10-bit HDR file, THEN the system SHALL output a 10-bit HDR10 file
2. WHEN the system applies tonemapping during DV stripping, THEN the system SHALL use the Hable tonemapping algorithm with zero desaturation
3. WHEN the system converts color spaces during DV stripping, THEN the system SHALL use high precision floating-point intermediate format
4. WHEN the system completes DV stripping, THEN the system SHALL continue with normal pixel format conversion and hardware upload

### Requirement 5

**User Story:** As a system operator, I want clear logging when Dolby Vision is detected and stripped, so that I can verify the system is handling DV content correctly.

#### Acceptance Criteria

1. WHEN Dolby Vision is detected during file probing, THEN the system SHALL log a warning with the message "Dolby Vision detected - will be stripped to prevent corruption"
2. WHEN the system begins stripping Dolby Vision metadata, THEN the system SHALL log an informational message with the message "Stripping Dolby Vision metadata to prevent encoding corruption"
3. WHEN encoding parameters are logged for a DV file, THEN the system SHALL include "DV: strip" in the parameter summary

### Requirement 6

**User Story:** As a developer, I want comprehensive test coverage for Dolby Vision detection, so that I can ensure the detection logic works correctly across various input formats.

#### Acceptance Criteria

1. WHEN the test suite runs, THEN the system SHALL verify that files with SMPTE ST 2094 color transfer are detected as Dolby Vision
2. WHEN the test suite runs, THEN the system SHALL verify that files with DV codec tags are detected as Dolby Vision
3. WHEN the test suite runs, THEN the system SHALL verify that files with DV stream tags are detected as Dolby Vision
4. WHEN the test suite runs, THEN the system SHALL verify that files without any DV markers are not detected as Dolby Vision
5. WHEN the test suite runs, THEN the system SHALL verify that encoding parameters correctly include the DV detection flag
