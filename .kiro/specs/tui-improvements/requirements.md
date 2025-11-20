# Requirements Document

## Introduction

This document specifies requirements for significant improvements to the AV1 transcoding daemon TUI (Terminal User Interface). The goal is to create a beautiful, modern, clean interface that displays as much useful information as possible while maintaining excellent usability and visual clarity.

## Glossary

- **TUI**: Terminal User Interface - a text-based user interface that runs in a terminal
- **Job**: A transcoding task that converts a video file to AV1 format
- **Ratatui**: The Rust library used for building the TUI
- **System**: The TUI application
- **User**: A person interacting with the TUI
- **Job Table**: The main table displaying all transcoding jobs
- **Dashboard**: The overall TUI interface showing system stats, jobs, and controls

## Requirements

### Requirement 1: Enhanced Visual Design

**User Story:** As a user, I want a modern, visually appealing interface with clear visual hierarchy, so that I can quickly understand the system state and find information easily.

#### Acceptance Criteria

1. WHEN the TUI renders THEN the System SHALL use a consistent color scheme with distinct colors for different job statuses (pending=yellow, running=green, success=blue, failed=red, skipped=gray)
2. WHEN displaying job information THEN the System SHALL use Unicode box-drawing characters and symbols to create a polished, modern appearance
3. WHEN rendering UI sections THEN the System SHALL use appropriate spacing and borders to create clear visual separation between different areas
4. WHEN displaying numeric data THEN the System SHALL use color gradients or intensity to indicate relative values (e.g., higher percentages shown in brighter colors)
5. WHEN the terminal size changes THEN the System SHALL adapt the layout responsively to maintain readability

### Requirement 2: Comprehensive Job Information Display

**User Story:** As a user, I want to see detailed video metadata for each job, so that I can understand what is being transcoded and make informed decisions.

#### Acceptance Criteria

1. WHEN displaying a job in the table THEN the System SHALL show video resolution (width x height)
2. WHEN displaying a job in the table THEN the System SHALL show source codec information
3. WHEN displaying a job in the table THEN the System SHALL show bitrate information when available
4. WHEN displaying a job with HDR content THEN the System SHALL indicate HDR status with a visual indicator
5. WHEN displaying a job THEN the System SHALL show bit depth information (8-bit or 10-bit)
6. WHEN displaying completed jobs THEN the System SHALL show actual compression ratio achieved

### Requirement 3: Interactive Navigation and Filtering

**User Story:** As a user, I want to navigate through jobs and filter by status, so that I can focus on specific jobs of interest.

#### Acceptance Criteria

1. WHEN the user presses arrow keys THEN the System SHALL allow scrolling through the job list
2. WHEN the user selects a job THEN the System SHALL highlight the selected row with a distinct visual style
3. WHEN the user presses a filter key (1-5) THEN the System SHALL filter jobs by status (1=all, 2=pending, 3=running, 4=success, 5=failed)
4. WHEN a filter is active THEN the System SHALL display the current filter in the UI
5. WHEN the user presses 's' THEN the System SHALL cycle through sort options (by date, by size, by status, by savings)

### Requirement 4: Enhanced Progress Visualization

**User Story:** As a user, I want detailed progress information for running jobs, so that I can monitor transcoding progress accurately.

#### Acceptance Criteria

1. WHEN a job is running THEN the System SHALL display a multi-segment progress bar showing different stages (probing, transcoding, verifying)
2. WHEN displaying progress THEN the System SHALL show frame-level progress information when available
3. WHEN a job is transcoding THEN the System SHALL display current FPS (frames per second) processing rate
4. WHEN displaying ETA THEN the System SHALL show both time remaining and estimated completion time
5. WHEN a job stage changes THEN the System SHALL update the progress bar color to reflect the current stage

### Requirement 5: Summary Statistics Dashboard

**User Story:** As a user, I want to see aggregate statistics about all jobs, so that I can understand overall system performance and space savings.

#### Acceptance Criteria

1. WHEN the TUI renders THEN the System SHALL display total space saved across all completed jobs
2. WHEN displaying statistics THEN the System SHALL show average compression ratio achieved
3. WHEN displaying statistics THEN the System SHALL show total processing time across all jobs
4. WHEN displaying statistics THEN the System SHALL show estimated total space savings for pending jobs
5. WHEN displaying statistics THEN the System SHALL show success rate percentage

### Requirement 6: Detailed Job View

**User Story:** As a user, I want to view detailed information about a selected job, so that I can see all metadata and processing details.

#### Acceptance Criteria

1. WHEN the user presses Enter on a selected job THEN the System SHALL open a detailed view panel
2. WHEN displaying detailed view THEN the System SHALL show all video metadata (codec, resolution, bitrate, frame rate, bit depth, pixel format)
3. WHEN displaying detailed view THEN the System SHALL show complete job history (created, started, finished timestamps)
4. WHEN displaying detailed view THEN the System SHALL show full file paths
5. WHEN displaying detailed view THEN the System SHALL show encoding parameters used (quality, profile)
6. WHEN the user presses Escape or Enter THEN the System SHALL close the detailed view and return to the main table

### Requirement 7: Enhanced Current Job Display

**User Story:** As a user, I want comprehensive real-time information about the currently running job, so that I can monitor progress in detail.

#### Acceptance Criteria

1. WHEN a job is running THEN the System SHALL display video metadata in the current job panel
2. WHEN a job is running THEN the System SHALL show current processing speed in FPS
3. WHEN a job is running THEN the System SHALL display a visual representation of progress with multiple segments
4. WHEN a job is running THEN the System SHALL show estimated final file size
5. WHEN a job is running THEN the System SHALL display compression ratio being achieved

### Requirement 8: Improved Status Bar

**User Story:** As a user, I want a comprehensive status bar with helpful information and keyboard shortcuts, so that I can understand available actions and system state.

#### Acceptance Criteria

1. WHEN the TUI renders THEN the System SHALL display all available keyboard shortcuts in the status bar
2. WHEN displaying shortcuts THEN the System SHALL group related shortcuts together visually
3. WHEN a filter or sort is active THEN the System SHALL display the current filter/sort mode in the status bar
4. WHEN displaying the status bar THEN the System SHALL show the last refresh time
5. WHEN displaying the status bar THEN the System SHALL show the current refresh rate

### Requirement 9: Job History and Trends

**User Story:** As a user, I want to see historical trends and patterns in job processing, so that I can understand system performance over time.

#### Acceptance Criteria

1. WHEN displaying completed jobs THEN the System SHALL show processing time trends using sparkline visualization
2. WHEN displaying statistics THEN the System SHALL show space savings trends over time
3. WHEN displaying the dashboard THEN the System SHALL show recent job completion rate
4. WHEN displaying trends THEN the System SHALL use the last 20 completed jobs for calculations
5. WHEN no historical data is available THEN the System SHALL display a message indicating insufficient data

### Requirement 10: Responsive Layout Management

**User Story:** As a user, I want the interface to adapt intelligently to different terminal sizes, so that I can use it effectively on various displays.

#### Acceptance Criteria

1. WHEN the terminal width is less than 120 columns THEN the System SHALL hide less critical columns in the job table
2. WHEN the terminal height is less than 20 rows THEN the System SHALL reduce the size of the statistics panel
3. WHEN the terminal is very small (less than 80x12) THEN the System SHALL display a simplified view with essential information only
4. WHEN the terminal is large (more than 160 columns) THEN the System SHALL display additional columns with more detailed information
5. WHEN the layout changes THEN the System SHALL maintain the user's current selection and scroll position
