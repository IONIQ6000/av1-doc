# Implementation Plan

- [x] 1. Implement Dolby Vision detection in FFProbe module
  - Add `has_dolby_vision()` method to `FFProbeStream` struct with three detection methods
  - Add `has_dolby_vision()` method to `FFProbeData` struct for file-level detection
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5_

- [x] 1.1 Write property test for color transfer detection
  - **Property 1: Color Transfer Detection**
  - **Validates: Requirements 1.1**

- [x] 1.2 Write property test for stream tag detection
  - **Property 2: Stream Tag Detection**
  - **Validates: Requirements 1.2**

- [x] 1.3 Write property test for codec name detection
  - **Property 3: Codec Name Detection**
  - **Validates: Requirements 1.3**

- [x] 1.4 Write property test for multi-stream detection
  - **Property 4: Multi-Stream Detection**
  - **Validates: Requirements 1.4**

- [x] 1.5 Write property test for detection method independence
  - **Property 5: Detection Method Independence**
  - **Validates: Requirements 1.5**

- [x] 1.6 Write property test for no false positives
  - **Property 14: No False Positives**
  - **Validates: Requirements 6.4**

- [x] 2. Add Dolby Vision flag to encoding parameters
  - Add `has_dolby_vision` boolean field to `EncodingParams` struct
  - Update `determine_encoding_params()` to call DV detection and set flag
  - Add logging when DV is detected
  - _Requirements: 2.1, 2.2, 2.3_

- [x] 2.1 Write property test for encoding parameters DV flag
  - **Property 6: Encoding Parameters Include DV Flag**
  - **Validates: Requirements 2.1**

- [x] 3. Implement DV stripping filter chain
  - Modify `run_av1_qsv_job()` to check `has_dolby_vision` flag
  - Add DV stripping filters before standard format conversion when flag is true
  - Implement filter chain: linearization → float conversion → primary remapping → tonemapping → TV range conversion
  - Add logging when DV stripping is applied
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 4.2, 4.3, 4.4_

- [x] 3.1 Write property test for DV filter chain construction
  - **Property 7: DV Filter Chain Construction**
  - **Validates: Requirements 3.1**

- [x] 3.2 Write property test for linearization filter presence
  - **Property 8: Linearization Filter Presence**
  - **Validates: Requirements 3.2**

- [x] 3.3 Write property test for tonemapping filter presence
  - **Property 9: Tonemapping Filter Presence**
  - **Validates: Requirements 3.3**

- [x] 3.4 Write property test for BT.709 TV range conversion
  - **Property 10: BT.709 TV Range Conversion**
  - **Validates: Requirements 3.4**

- [x] 3.5 Write property test for Hable tonemapping parameters
  - **Property 11: Hable Tonemapping with Zero Desaturation**
  - **Validates: Requirements 4.2**

- [x] 3.6 Write property test for float format intermediate
  - **Property 12: Float Format Intermediate**
  - **Validates: Requirements 4.3**

- [x] 3.7 Write property test for filter chain ordering
  - **Property 13: Filter Chain Ordering**
  - **Validates: Requirements 4.4**

- [x] 4. Update existing tests to include DV flag
  - Update all test helper functions to include `has_dolby_vision: false` in `EncodingParams`
  - Update `create_test_metadata()` helper to support DV markers
  - Ensure all existing property tests pass with new field
  - _Requirements: 6.5_

- [x] 5. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.
