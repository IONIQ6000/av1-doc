# Requirements Document

## Introduction

This feature migrates the video encoding application from Docker-based FFmpeg execution to native software-only AV1 encoding using FFmpeg 8.0 or later. The system will encode AV1 on CPU using software encoders (libsvtav1, libaom-av1, librav1e) with a quality-first approach that prioritizes perceptual quality over compression efficiency or encoding speed.

## Glossary

- **FFmpeg**: A complete, cross-platform solution to record, convert and stream audio and video
- **AV1**: AOMedia Video 1, a royalty-free video coding format designed for video transmissions over the Internet
- **SVT-AV1**: Scalable Video Technology for AV1, an AV1 encoder/decoder implementation
- **SVT-AV1-PSY**: A perceptually-tuned fork of SVT-AV1 optimized for grain retention and visual quality
- **CRF**: Constant Rate Factor, a quality-based encoding mode where lower values mean higher quality
- **Remux**: A video file that has been extracted from physical media without re-encoding
- **WEB-DL**: A video file downloaded from a streaming service, already delivery-encoded
- **Film Grain**: Natural texture in film that should be preserved during encoding
- **Encoder Binary**: The executable FFmpeg program file
- **Source Classification**: The process of categorizing input video by quality tier
- **Test Clip**: A short segment extracted from source video for quality validation before full encode

## Requirements

### Requirement 1

**User Story:** As a system administrator, I want the application to use a local FFmpeg 8.0+ installation instead of Docker, so that I can avoid Docker dependencies and have direct control over the FFmpeg binary.

#### Acceptance Criteria

1. WHEN the application starts THEN the Encoder Binary SHALL execute FFmpeg commands directly via subprocess without Docker containers
2. WHEN the application initializes THEN the Encoder Binary SHALL verify FFmpeg version is 8.0 or greater
3. IF FFmpeg version is less than 8.0 or FFmpeg is not found THEN the Encoder Binary SHALL terminate with a clear error message indicating the version requirement
4. WHERE a user specifies FFMPEG_BIN configuration THEN the Encoder Binary SHALL use the specified path instead of the default PATH lookup
5. WHEN FFmpeg execution fails THEN the Encoder Binary SHALL report the error without Docker-related diagnostics

### Requirement 2

**User Story:** As a developer, I want the application to automatically detect and select the best available AV1 software encoder, so that encoding uses the highest quality encoder available on the system.

#### Acceptance Criteria

1. WHEN the application starts THEN the Encoder Binary SHALL query FFmpeg for available AV1 encoders using the encoders list command
2. WHEN multiple AV1 encoders are available THEN the Encoder Binary SHALL select encoders in priority order: SVT-AV1-PSY, libsvtav1, libaom-av1, librav1e
3. IF no AV1 software encoder is detected THEN the Encoder Binary SHALL terminate with an error message listing required encoder libraries
4. WHEN encoder selection completes THEN the Encoder Binary SHALL log which encoder was selected for the session
5. WHERE SVT-AV1-PSY is detected THEN the Encoder Binary SHALL enable perceptual tuning parameters by default

### Requirement 3

**User Story:** As a video archivist, I want the system to classify my source video by quality tier, so that encoding parameters are optimized for the specific source type.

#### Acceptance Criteria

1. WHEN a video file is queued for encoding THEN the Source Classification SHALL analyze bitrate, codec, and resolution to determine quality tier
2. WHEN source bitrate exceeds 15 Mbps for 1080p or 40 Mbps for 2160p THEN the Source Classification SHALL categorize as REMUX tier
3. WHEN source codec is HEVC, AV1, or VP9 with clean visual quality THEN the Source Classification SHALL categorize as WEB-DL tier
4. WHEN source shows visible compression artifacts or bitrate is below 5 Mbps for 1080p THEN the Source Classification SHALL categorize as LOW-QUALITY tier
5. IF classification is uncertain THEN the Source Classification SHALL default to the higher quality tier to avoid quality loss

### Requirement 4

**User Story:** As a video archivist, I want REMUX-tier sources to undergo test clip validation before full encoding, so that I can verify quality settings preserve grain and detail without artifacts.

#### Acceptance Criteria

1. WHEN a REMUX-tier source is queued THEN the Encoder Binary SHALL extract a 30-60 second Test Clip before starting full encode
2. WHEN selecting Test Clip segments THEN the Encoder Binary SHALL prioritize scenes containing darkness, grain, texture, and high motion
3. WHEN Test Clip encoding completes THEN the Encoder Binary SHALL pause and request user review before proceeding
4. IF user reports artifacts in Test Clip THEN the Encoder Binary SHALL lower CRF by 2 points or reduce preset speed by one step
5. WHEN user approves Test Clip quality THEN the Encoder Binary SHALL proceed with full encode using identical parameters

### Requirement 5

**User Story:** As a video archivist, I want REMUX-tier encodes to use quality-first CRF settings, so that the output preserves all grain, texture, and gradients from the source.

#### Acceptance Criteria

1. WHEN encoding 1080p REMUX sources THEN the Encoder Binary SHALL use CRF 18 as the starting quality value
2. WHEN encoding 2160p REMUX sources THEN the Encoder Binary SHALL use CRF 20 as the starting quality value
3. WHEN encoding REMUX sources THEN the Encoder Binary SHALL use SVT-AV1 preset 3 or slower
4. WHERE source contains visible film grain THEN the Encoder Binary SHALL enable film-grain synthesis with parameter value 8
5. WHERE SVT-AV1-PSY is available THEN the Encoder Binary SHALL enable tune=3 for grain-optimized encoding

### Requirement 6

**User Story:** As a media manager, I want WEB-DL sources to be re-encoded conservatively or skipped, so that I avoid compounding existing compression artifacts.

#### Acceptance Criteria

1. WHEN a WEB-DL source is already encoded with HEVC, AV1, or VP9 THEN the Encoder Binary SHALL skip re-encoding unless explicitly requested by user
2. WHEN re-encoding H.264 WEB-DL at 1080p THEN the Encoder Binary SHALL use CRF 26 as the starting quality value
3. WHEN re-encoding H.264 WEB-DL at 2160p THEN the Encoder Binary SHALL use CRF 28 as the starting quality value
4. WHEN encoding WEB-DL sources THEN the Encoder Binary SHALL use SVT-AV1 preset 5 as default
5. WHEN encoding WEB-DL sources THEN the Encoder Binary SHALL disable film-grain synthesis to avoid introducing artificial texture

### Requirement 7

**User Story:** As a media manager, I want LOW-QUALITY sources to be encoded with size-reduction settings, so that storage is optimized for already-degraded content.

#### Acceptance Criteria

1. WHEN encoding LOW-QUALITY sources THEN the Encoder Binary SHALL use CRF 30 as the starting quality value
2. WHEN encoding LOW-QUALITY sources THEN the Encoder Binary SHALL use SVT-AV1 preset 6 or faster
3. WHEN encoding LOW-QUALITY sources THEN the Encoder Binary SHALL disable film-grain synthesis
4. WHEN encoding LOW-QUALITY sources THEN the Encoder Binary SHALL skip Test Clip workflow
5. WHEN encoding LOW-QUALITY sources THEN the Encoder Binary SHALL prioritize encoding speed over quality optimization

### Requirement 8

**User Story:** As a video archivist, I want 10-bit and HDR sources to maintain their bit depth through encoding, so that color information and dynamic range are preserved.

#### Acceptance Criteria

1. WHEN source video is 10-bit color depth THEN the Encoder Binary SHALL output 10-bit AV1 using yuv420p10le pixel format
2. WHEN source video contains HDR metadata THEN the Encoder Binary SHALL preserve HDR metadata in the output container
3. WHEN processing 10-bit sources THEN the Encoder Binary SHALL use p010le or yuv420p10le in the filter chain
4. WHEN source video is 8-bit THEN the Encoder Binary SHALL output 8-bit AV1 without upconverting
5. WHEN bit depth detection fails THEN the Encoder Binary SHALL default to 10-bit output to avoid quality loss

### Requirement 9

**User Story:** As a system administrator, I want the application to generate correct FFmpeg command lines for each quality tier, so that encoding executes with appropriate quality-first parameters.

#### Acceptance Criteria

1. WHEN generating encode commands THEN the Encoder Binary SHALL include CRF value appropriate to source classification tier
2. WHEN generating encode commands THEN the Encoder Binary SHALL include preset value appropriate to source classification tier
3. WHEN generating REMUX encode commands THEN the Encoder Binary SHALL include film-grain and tune parameters
4. WHEN generating encode commands THEN the Encoder Binary SHALL copy audio and subtitle streams without re-encoding
5. WHEN generating encode commands THEN the Encoder Binary SHALL apply format conversion filters before encoder input

### Requirement 10

**User Story:** As a developer, I want all Docker-related code removed from the application, so that the codebase is simplified and Docker is no longer a dependency.

#### Acceptance Criteria

1. WHEN the application is built THEN the Encoder Binary SHALL contain no Docker client library dependencies
2. WHEN the application executes THEN the Encoder Binary SHALL not invoke docker run, docker pull, or docker build commands
3. WHEN the application initializes THEN the Encoder Binary SHALL not check for Docker daemon availability
4. WHEN encoding starts THEN the Encoder Binary SHALL not create or manage container lifecycle
5. WHEN the application is deployed THEN the Encoder Binary SHALL function on systems without Docker installed

### Requirement 11

**User Story:** As a system administrator, I want clear documentation on how to supply FFmpeg 8.0+ with required encoders, so that I can prepare the system for software AV1 encoding.

#### Acceptance Criteria

1. WHEN installation documentation is provided THEN the Encoder Binary SHALL document the FFmpeg 8.0+ version requirement
2. WHEN installation documentation is provided THEN the Encoder Binary SHALL list required encoder libraries: libsvtav1, libaom, librav1e
3. WHEN installation documentation is provided THEN the Encoder Binary SHALL provide FFmpeg build flags for enabling AV1 encoders
4. WHERE the application bundles FFmpeg THEN the Encoder Binary SHALL document the bundled binary location and version
5. WHEN installation documentation is provided THEN the Encoder Binary SHALL explain the FFMPEG_BIN configuration option

### Requirement 12

**User Story:** As a video archivist, I want the prime directive of maximum perceptual quality enforced throughout the encoding pipeline, so that quality is never sacrificed for efficiency or file size.

#### Acceptance Criteria

1. WHEN any encoding decision involves a tradeoff THEN the Encoder Binary SHALL choose the option that maximizes perceptual quality
2. WHEN CRF values are selected THEN the Encoder Binary SHALL prefer lower CRF values that increase quality even if file size increases
3. WHEN preset values are selected THEN the Encoder Binary SHALL prefer slower presets that improve quality even if encoding time increases
4. IF user does not explicitly request size optimization THEN the Encoder Binary SHALL not adjust parameters to reduce file size
5. WHEN quality versus efficiency conflicts arise THEN the Encoder Binary SHALL log the decision and explain why quality was prioritized
