# Implementation Plan

- [x] 1. Update configuration defaults and Docker image
  - Update `TranscodeConfig::default_config()` to use new Docker image
  - Change default `docker_image` from `ghcr.io/linuxserver/ffmpeg:latest` to `lscr.io/linuxserver/ffmpeg:version-8.0-cli`
  - _Requirements: 1.5_

- [x] 2. Update encoding parameter determination for QSV profile handling
  - Modify `determine_encoding_params()` function
  - Change AV1 profile selection: always use profile 0 for QSV (both 8-bit and 10-bit)
  - Keep pixel format selection unchanged (nv12 for 8-bit, p010le for 10-bit)
  - Update logging to reflect QSV profile usage
  - _Requirements: 2.1, 2.2, 2.4_

- [x] 2.1 Write property test for profile selection
  - **Property 6: QSV Profile for 10-bit**
  - **Validates: Requirements 2.2**

- [x] 2.2 Write property test for pixel format selection
  - **Property 5 & 7: Pixel Format Selection**
  - **Validates: Requirements 2.1, 2.4**

- [x] 3. Rename and refactor main encoding function from VAAPI to QSV
  - Rename `run_av1_vaapi_job()` to `run_av1_qsv_job()`
  - Update function documentation to reference QSV instead of VAAPI
  - Keep function signature unchanged
  - _Requirements: 1.1, 1.2, 1.3_

- [x] 4. Update Docker command construction for QSV
  - Add LIBVA_DRIVER_NAME environment variable to Docker command
  - Use `-e LIBVA_DRIVER_NAME=iHD` argument
  - Keep existing Docker arguments (--rm, --privileged, --user root, --entrypoint)
  - _Requirements: 1.4_

- [x] 4.1 Write property test for environment variable
  - **Property 4: LIBVA Driver Environment Variable**
  - **Validates: Requirements 1.4**

- [x] 5. Update hardware device initialization for QSV
  - Replace `-init_hw_device vaapi=va:/dev/dri/renderD128` with `-init_hw_device qsv=hw:/dev/dri/renderD128`
  - Add `-filter_hw_device hw` argument after hardware device initialization
  - Remove `-hwaccel vaapi` and `-hwaccel_device` arguments (not needed for QSV)
  - _Requirements: 1.1, 3.1_

- [x] 5.1 Write property test for QSV initialization
  - **Property 1: QSV Hardware Initialization**
  - **Validates: Requirements 1.1, 3.1**

- [x] 5.2 Write property test for device path
  - **Property 8: Device Path in Initialization**
  - **Validates: Requirements 3.1**

- [x] 6. Update video filter chain for QSV compatibility
  - Keep format conversion filters (pad, setsar, format)
  - Change `hwupload=extra_hw_frames=64` to just `hwupload` (QSV doesn't need extra_hw_frames)
  - Ensure format filter uses correct pixel format (nv12 or p010le) based on bit depth
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [x] 6.1 Write property test for filter chain ordering
  - **Property 12: Filter Chain Ordering**
  - **Validates: Requirements 5.1**

- [x] 6.2 Write property test for hwupload parameters
  - **Property 13: QSV HWUpload Filter**
  - **Validates: Requirements 5.2**

- [x] 6.3 Write property test for 10-bit filter chain
  - **Property 14: 10-bit Filter Chain**
  - **Validates: Requirements 5.3**

- [x] 6.4 Write property test for 8-bit filter chain
  - **Property 15: 8-bit Filter Chain**
  - **Validates: Requirements 5.4**

- [x] 7. Update video codec and quality parameters
  - Replace `-c:v av1_vaapi` with `-c:v av1_qsv`
  - Replace `-qp <value>` with `-global_quality <value>`
  - Update `-profile:v` to use "main" string instead of numeric value for QSV
  - Keep quality_used value in return result
  - _Requirements: 1.2, 1.3, 4.2, 4.4_

- [x] 7.1 Write property test for codec selection
  - **Property 2: AV1 QSV Codec Selection**
  - **Validates: Requirements 1.2**

- [x] 7.2 Write property test for quality parameter
  - **Property 3: Global Quality Parameter**
  - **Validates: Requirements 1.3, 4.2**

- [x] 7.3 Write property test for quality value in result
  - **Property 11: Quality Value in Result**
  - **Validates: Requirements 4.4**

- [x] 8. Update debug logging for QSV
  - Update log messages to reference QSV instead of VAAPI
  - Log QSV-specific initialization parameters
  - Keep existing log format for compatibility
  - _Requirements: 6.3_

- [x] 9. Update function call sites to use new QSV function
  - Find all calls to `run_av1_vaapi_job()`
  - Replace with calls to `run_av1_qsv_job()`
  - Verify no other code changes needed at call sites
  - _Requirements: 1.1_

- [x] 9.1 Write property test for quality calculation preservation
  - **Property 10: Quality Calculation Preservation**
  - **Validates: Requirements 4.1**

- [x] 10. Update error handling and messages for QSV
  - Update error messages to reference QSV instead of VAAPI
  - Include device path in QSV initialization errors
  - Add context about QSV-specific issues in error messages
  - _Requirements: 3.4, 6.4_

- [x] 11. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 12. Manual integration testing
  - Test 8-bit H.264 source encoding to AV1 with QSV
  - Test 10-bit HEVC source encoding to AV1 with QSV
  - Verify output pixel format (yuv420p for 8-bit, yuv420p10le for 10-bit)
  - Test with actual Intel Arc GPU hardware
  - Verify encoding speed and GPU utilization
  - _Requirements: 2.3, 2.5, 7.1, 7.2, 7.3, 7.4_

- [ ] 13. Update documentation
  - Update README or documentation to mention QSV requirement
  - Document Docker image requirement
  - Document Intel Arc GPU compatibility
  - Add troubleshooting section for QSV issues
  - _Requirements: 6.1, 6.2_

- [ ] 14. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.
