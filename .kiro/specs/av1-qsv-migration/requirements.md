# Requirements Document

## Introduction

This specification addresses the migration of AV1 hardware encoding from VAAPI to Intel QSV (Quick Sync Video) to enable 10-bit AV1 encoding support on Intel Arc GPUs. Current testing has revealed that while VAAPI supports 8-bit AV1 encoding, it does not expose 10-bit AV1 encoding capabilities. Intel QSV provides full support for both 8-bit and 10-bit AV1 hardware encoding on Intel Arc A310/A380 GPUs.

## Glossary

- **QSV**: Intel Quick Sync Video - Intel's hardware video encoding/decoding technology
- **VAAPI**: Video Acceleration API - Linux API for hardware video acceleration
- **AV1**: AOMedia Video 1 - Modern video codec with superior compression
- **Bit Depth**: Number of bits used to represent color information (8-bit or 10-bit)
- **Pixel Format**: Format for storing pixel data (nv12 for 8-bit, p010le for 10-bit)
- **Global Quality**: QSV quality parameter (equivalent to QP in VAAPI)
- **Docker Container**: Isolated environment running the ffmpeg encoder
- **Intel Arc GPU**: Intel's discrete graphics card with hardware AV1 encoding support

## Requirements

### Requirement 1

**User Story:** As a system operator, I want the transcoding system to use Intel QSV for AV1 encoding, so that both 8-bit and 10-bit content can be hardware-encoded efficiently.

#### Acceptance Criteria

1. WHEN the system initializes hardware encoding THEN the system SHALL use QSV device initialization instead of VAAPI device initialization
2. WHEN the system builds ffmpeg commands THEN the system SHALL use the av1_qsv codec instead of av1_vaapi codec
3. WHEN the system sets quality parameters THEN the system SHALL use global_quality instead of qp parameter
4. WHEN the system runs Docker containers THEN the system SHALL set the LIBVA_DRIVER_NAME environment variable to iHD
5. WHEN the system runs Docker containers THEN the system SHALL use the lscr.io/linuxserver/ffmpeg:version-8.0-cli image

### Requirement 2

**User Story:** As a system operator, I want 10-bit source content to be encoded as 10-bit AV1, so that color depth and HDR information are preserved.

#### Acceptance Criteria

1. WHEN the system detects 10-bit source content THEN the system SHALL use p010le pixel format for encoding
2. WHEN the system encodes 10-bit content THEN the system SHALL set AV1 profile to main (profile 0 for QSV)
3. WHEN the system completes 10-bit encoding THEN the output SHALL have yuv420p10le pixel format
4. WHEN the system detects 8-bit source content THEN the system SHALL use nv12 pixel format for encoding
5. WHEN the system encodes 8-bit content THEN the output SHALL have yuv420p pixel format

### Requirement 3

**User Story:** As a system operator, I want the hardware device path to be correctly specified for QSV, so that the Intel Arc GPU is properly utilized.

#### Acceptance Criteria

1. WHEN the system initializes QSV hardware THEN the system SHALL specify the device path as /dev/dri/renderD128
2. WHEN the system mounts Docker volumes THEN the system SHALL mount /dev/dri as a device
3. WHEN the system uses hardware upload filters THEN the system SHALL reference the QSV hardware device
4. WHEN hardware initialization fails THEN the system SHALL log an error with device path information

### Requirement 4

**User Story:** As a system operator, I want quality parameters to be correctly translated from QP to global_quality, so that encoding quality remains consistent after migration.

#### Acceptance Criteria

1. WHEN the system calculates optimal quality THEN the system SHALL use the same QP calculation logic
2. WHEN the system applies quality to QSV encoding THEN the system SHALL use global_quality parameter with the calculated QP value
3. WHEN the system logs encoding parameters THEN the system SHALL report the quality value used
4. WHEN the system completes encoding THEN the system SHALL return the quality value used in the result

### Requirement 5

**User Story:** As a system operator, I want the video filter chain to work correctly with QSV, so that video processing and hardware upload function properly.

#### Acceptance Criteria

1. WHEN the system builds filter chains THEN the system SHALL use format conversion before hwupload
2. WHEN the system uploads to hardware THEN the system SHALL use hwupload filter without extra_hw_frames parameter for QSV
3. WHEN the system processes 10-bit content THEN the system SHALL convert to p010le format before hardware upload
4. WHEN the system processes 8-bit content THEN the system SHALL convert to nv12 format before hardware upload

### Requirement 6

**User Story:** As a developer, I want the code to maintain backward compatibility with existing configuration, so that the migration does not break existing deployments.

#### Acceptance Criteria

1. WHEN the system reads configuration THEN the system SHALL continue to use existing docker_image field if not updated
2. WHEN the system reads configuration THEN the system SHALL continue to use existing gpu_device field
3. WHEN the system initializes THEN the system SHALL log which encoding method is being used (QSV vs VAAPI)
4. WHEN the system encounters errors THEN the system SHALL provide clear error messages indicating QSV-specific issues

### Requirement 7

**User Story:** As a system operator, I want encoding performance to be maintained or improved, so that transcoding throughput remains high.

#### Acceptance Criteria

1. WHEN the system encodes 8-bit content with QSV THEN the encoding speed SHALL be comparable to or faster than VAAPI
2. WHEN the system encodes 10-bit content with QSV THEN the encoding SHALL use hardware acceleration
3. WHEN the system monitors encoding progress THEN the system SHALL report frames per second and speed metrics
4. WHEN the system completes encoding THEN the system SHALL log total elapsed time and average speed
