# Implementation Plan

- [x] 1. Refactor App state structure and add new state management
  - Extract UI state into separate UiState struct with selection, filter, sort, and view mode
  - Add StatisticsCache struct for aggregate metrics
  - Add ColorScheme struct with consistent color definitions
  - Update App struct to include new state components
  - _Requirements: 1.1, 3.3, 3.4, 3.5, 5.1, 5.2, 5.3, 5.4, 5.5_

- [x] 2. Implement filtering and sorting logic
  - [x] 2.1 Create JobFilter enum and filtering functions
    - Define JobFilter enum (All, Pending, Running, Success, Failed)
    - Implement filter_jobs function that returns filtered job list
    - Add keyboard handler for filter keys (1-5)
    - _Requirements: 3.3, 3.4_

  - [x] 2.2 Create SortMode enum and sorting functions
    - Define SortMode enum (ByDate, BySize, ByStatus, BySavings)
    - Implement sort_jobs function for each mode
    - Add keyboard handler for 's' key to cycle sort modes
    - _Requirements: 3.5_

  - [x] 2.3 Write property test for filtering
    - **Property 1: Filter consistency**
    - **Validates: Requirements 3.3**

  - [x] 2.4 Write property test for sorting
    - **Property 2: Sort order consistency**
    - **Validates: Requirements 3.5**

- [x] 3. Implement navigation and selection
  - [x] 3.1 Add selection state management
    - Add selected_index and scroll_offset to UiState
    - Implement selection movement functions (up, down, page up, page down)
    - Add keyboard handlers for arrow keys
    - _Requirements: 3.1, 3.2_

  - [x] 3.2 Implement visual selection highlighting
    - Update render_job_table to highlight selected row
    - Use distinct border color for selected row
    - Ensure selection is visible when scrolling
    - _Requirements: 3.2_

  - [x] 3.3 Write property test for selection bounds
    - **Property 3: Selection bounds**
    - **Validates: Requirements 3.2**

- [x] 4. Implement statistics calculation and caching
  - [x] 4.1 Create StatisticsCache struct and calculation logic
    - Implement calculate_statistics function for aggregate metrics
    - Calculate total space saved, average compression ratio, success rate
    - Calculate estimated pending savings
    - Calculate total processing time
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5_

  - [x] 4.2 Add trend calculation for recent jobs
    - Extract last 20 completed jobs for trend analysis
    - Calculate processing time trends
    - Calculate compression ratio trends
    - Calculate recent completion rate
    - _Requirements: 9.1, 9.2, 9.3, 9.4_

  - [x] 4.3 Write unit tests for statistics calculation
    - Test total space saved with various job sets
    - Test average compression ratio calculation
    - Test success rate calculation
    - Test edge cases (empty list, all failed, etc.)
    - _Requirements: 5.1, 5.2, 5.5_

  - [x] 4.4 Write property test for statistics accuracy
    - **Property 4: Statistics accuracy**
    - **Validates: Requirements 5.1**

- [x] 5. Create statistics dashboard component
  - [x] 5.1 Implement render_statistics_dashboard function
    - Create layout for statistics panel
    - Display total space saved with formatting
    - Display average compression ratio
    - Display success rate percentage
    - Display estimated pending savings
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5_

  - [x] 5.2 Add sparkline visualization for trends
    - Implement sparkline rendering for processing times
    - Implement sparkline rendering for compression ratios
    - Add trend indicators (up/down arrows)
    - _Requirements: 9.1, 9.2_

  - [x] 5.3 Add conditional rendering based on terminal size
    - Show full statistics panel when height >= 20
    - Show compact statistics when height < 20
    - Hide statistics when height < 15
    - _Requirements: 10.2_

- [x] 6. Enhance job table with additional columns
  - [x] 6.1 Add resolution column
    - Extract width and height from job metadata
    - Format as "WIDTHxHEIGHT" (e.g., "1920x1080")
    - Show "-" when metadata not available
    - _Requirements: 2.1_

  - [x] 6.2 Add codec column
    - Display source codec name
    - Use color coding for different codecs
    - Show "-" when not available
    - _Requirements: 2.2_

  - [x] 6.3 Add bitrate column
    - Format bitrate in Mbps
    - Show "-" when not available
    - _Requirements: 2.3_

  - [x] 6.4 Add HDR indicator
    - Show "HDR" badge when is_hdr is true
    - Use distinct color for HDR indicator
    - _Requirements: 2.4_

  - [x] 6.5 Add bit depth indicator
    - Show "8-bit" or "10-bit" based on source_bit_depth
    - Show "-" when not available
    - _Requirements: 2.5_

  - [x] 6.6 Add compression ratio column
    - Calculate and display actual compression ratio for completed jobs
    - Format as percentage (e.g., "45%")
    - _Requirements: 2.6_

  - [x] 6.7 Write property test for compression ratio calculation
    - **Property 8: Compression ratio calculation**
    - **Validates: Requirements 2.6**

- [x] 7. Implement responsive column selection
  - [x] 7.1 Create LayoutConfig struct
    - Define TableColumn enum for all possible columns
    - Implement from_terminal_size to determine visible columns
    - Add logic for small (< 120), medium (120-160), large (> 160) terminals
    - _Requirements: 10.1, 10.4_

  - [x] 7.2 Update render_job_table to use LayoutConfig
    - Dynamically build table header based on visible columns
    - Dynamically build table rows based on visible columns
    - Adjust column widths based on available space
    - _Requirements: 10.1, 10.4_

  - [x] 7.3 Write property test for layout responsiveness
    - **Property 7: Layout responsiveness**
    - **Validates: Requirements 1.5, 10.5**

- [x] 8. Enhance current job panel with detailed information
  - [x] 8.1 Add video metadata display
    - Show resolution, codec, bitrate in current job panel
    - Show HDR status and bit depth
    - Format information clearly with labels
    - _Requirements: 7.1_

  - [x] 8.2 Add FPS processing rate display
    - Calculate current FPS from progress tracking
    - Display with appropriate formatting
    - Show "-" when not available
    - _Requirements: 7.2_

  - [x] 8.3 Add multi-segment progress bar
    - Create progress bar with different colors for stages
    - Show stage transitions visually
    - Update colors based on current stage
    - _Requirements: 7.3, 4.5_

  - [x] 8.4 Add estimated final size display
    - Calculate estimated final size based on progress
    - Display alongside current size
    - _Requirements: 7.4_

  - [x] 8.5 Add current compression ratio display
    - Calculate ratio from current temp file size vs original
    - Display as percentage
    - _Requirements: 7.5_

- [x] 9. Implement detail view modal
  - [x] 9.1 Create detail view rendering function
    - Create modal overlay that covers center of screen
    - Add border and title
    - Implement scrolling for long content
    - _Requirements: 6.1_

  - [x] 9.2 Display comprehensive job metadata
    - Show all video metadata fields
    - Show encoding parameters (quality, profile)
    - Show bit depth and pixel format
    - _Requirements: 6.2, 6.5_

  - [x] 9.3 Display complete job history
    - Show created, started, finished timestamps
    - Calculate and show durations
    - Format timestamps clearly
    - _Requirements: 6.3_

  - [x] 9.4 Display full file paths
    - Show complete source path
    - Show output path if available
    - Handle long paths with wrapping
    - _Requirements: 6.4_

  - [x] 9.5 Add keyboard handlers for detail view
    - Handle Enter key to open detail view
    - Handle Escape/Enter to close detail view
    - Update view mode state appropriately
    - _Requirements: 6.6_

  - [x] 9.6 Write property test for detail view completeness
    - **Property 10: Detail view data completeness**
    - **Validates: Requirements 6.2, 6.5**

- [x] 10. Implement enhanced color scheme
  - [x] 10.1 Create ColorScheme struct with all colors
    - Define colors for all job statuses
    - Define colors for UI elements (borders, text)
    - Define colors for progress stages
    - Define colors for metrics (low, medium, high)
    - _Requirements: 1.1_

  - [x] 10.2 Apply color scheme throughout UI
    - Update all rendering functions to use ColorScheme
    - Apply status colors to job rows
    - Apply border colors based on selection
    - Apply progress colors based on stage
    - _Requirements: 1.1, 1.4_

  - [x] 10.3 Write property test for color consistency
    - **Property 6: Color scheme consistency**
    - **Validates: Requirements 1.1**

- [x] 11. Improve visual design with Unicode and spacing
  - [x] 11.1 Add Unicode symbols and box-drawing characters
    - Use Unicode symbols for status indicators (✓, ✗, ⚙, ⏸)
    - Use box-drawing characters for borders
    - Use arrows and indicators for trends
    - _Requirements: 1.2_

  - [x] 11.2 Improve spacing and visual hierarchy
    - Add appropriate padding in panels
    - Use consistent spacing between sections
    - Create clear visual separation with borders
    - _Requirements: 1.3_

  - [x] 11.3 Add color gradients for numeric values
    - Implement gradient calculation for percentages
    - Apply to progress bars, metrics, statistics
    - Use intensity to indicate relative values
    - _Requirements: 1.4_

- [x] 12. Update status bar with comprehensive information
  - [x] 12.1 Display all keyboard shortcuts
    - Group shortcuts by category (navigation, actions, views)
    - Format clearly with separators
    - _Requirements: 8.1, 8.2_

  - [x] 12.2 Display current filter and sort mode
    - Show active filter in status bar
    - Show active sort mode in status bar
    - Use distinct formatting for active modes
    - _Requirements: 8.3_

  - [x] 12.3 Display refresh information
    - Show last refresh time
    - Show current refresh rate
    - _Requirements: 8.4, 8.5_

- [x] 13. Implement enhanced progress tracking
  - [x] 13.1 Add frame-level progress tracking
    - Extract frame information from ffmpeg output if available
    - Calculate frames processed and total frames
    - Calculate current FPS
    - _Requirements: 4.2, 4.3_

  - [x] 13.2 Enhance JobProgress struct
    - Add frames_processed, total_frames, current_fps fields
    - Add estimated_final_size field
    - Add current_compression_ratio field
    - Update progress detection to populate new fields
    - _Requirements: 4.2, 4.3, 7.4, 7.5_

  - [x] 13.3 Write property test for progress bounds
    - **Property 5: Progress percentage bounds**
    - **Validates: Requirements 4.1**

- [x] 14. Implement responsive layout management
  - [x] 14.1 Create layout calculation function
    - Determine component visibility based on terminal size
    - Calculate component heights and widths
    - Handle minimum size requirements
    - _Requirements: 10.1, 10.2, 10.3, 10.4_

  - [x] 14.2 Update main UI function to use responsive layout
    - Call layout calculation at start of rendering
    - Conditionally render components based on layout config
    - Maintain scroll position during layout changes
    - _Requirements: 10.5_

  - [x] 14.3 Add simplified view for very small terminals
    - Create minimal layout for terminals < 80x12
    - Show only essential information
    - Display clear message about limited space
    - _Requirements: 10.3_

- [x] 15. Add keyboard shortcut handling
  - [x] 15.1 Implement filter key handlers (1-5)
    - Map keys to filter modes
    - Update UI state when filter changes
    - _Requirements: 3.3, 3.4_

  - [x] 15.2 Implement sort key handler ('s')
    - Cycle through sort modes
    - Update UI state when sort changes
    - _Requirements: 3.5_

  - [x] 15.3 Implement navigation key handlers (arrows, page up/down)
    - Handle up/down arrows for selection
    - Handle page up/down for fast scrolling
    - Update selection and scroll offset
    - _Requirements: 3.1_

  - [x] 15.4 Implement detail view key handlers (Enter, Escape)
    - Open detail view on Enter
    - Close detail view on Escape or Enter
    - _Requirements: 6.1, 6.6_

  - [x] 15.5 Write property test for keyboard shortcut uniqueness
    - **Property 9: Keyboard shortcut uniqueness**
    - **Validates: Requirements 8.1**

- [x] 16. Final integration and polish
  - [x] 16.1 Integrate all components into main UI
    - Wire up all rendering functions
    - Ensure proper layout and spacing
    - Test all interactions
    - _Requirements: All_

  - [x] 16.2 Performance optimization
    - Profile rendering performance with many jobs
    - Optimize statistics calculation caching
    - Optimize progress tracking updates
    - _Requirements: All_

  - [x] 16.3 Error handling and edge cases
    - Handle empty job lists gracefully
    - Handle missing metadata gracefully
    - Handle terminal resize during rendering
    - _Requirements: All_

  - [x] 16.4 Write integration tests
    - Test full rendering pipeline
    - Test all keyboard shortcuts
    - Test state transitions
    - Test refresh cycle
    - _Requirements: All_

- [x] 17. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.
