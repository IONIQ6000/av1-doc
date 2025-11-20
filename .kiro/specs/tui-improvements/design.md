# Design Document: TUI Improvements

## Overview

This design document outlines the architecture and implementation approach for significantly improving the AV1 transcoding daemon TUI. The improvements focus on creating a modern, information-dense, and highly usable terminal interface that provides comprehensive visibility into the transcoding system.

The design builds upon the existing Ratatui-based TUI, enhancing it with:
- Rich visual design with consistent color schemes and modern styling
- Comprehensive job information display with video metadata
- Interactive navigation with filtering and sorting capabilities
- Enhanced progress visualization with multi-stage indicators
- Summary statistics dashboard for aggregate insights
- Detailed job view for in-depth inspection
- Responsive layout that adapts to terminal size

## Architecture

### High-Level Structure

The TUI follows a component-based architecture with clear separation of concerns:

```
┌─────────────────────────────────────────────────────────┐
│                     Main Event Loop                      │
│  (Input handling, refresh timing, state management)     │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                      App State                           │
│  - Jobs list with metadata                              │
│  - System metrics (CPU, Memory, GPU)                    │
│  - UI state (selection, filter, sort, view mode)       │
│  - Statistics cache (aggregated metrics)                │
│  - Progress tracking (running jobs)                     │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                   Rendering Pipeline                     │
│                                                          │
│  ┌──────────────────────────────────────────────────┐  │
│  │  Layout Manager (responsive sizing)              │  │
│  └──────────────────────────────────────────────────┘  │
│                            │                             │
│  ┌─────────────┬──────────┴──────────┬──────────────┐  │
│  │             │                      │              │  │
│  ▼             ▼                      ▼              ▼  │
│  Header    Statistics            Job Table      Status  │
│  Panel     Dashboard              Component       Bar   │
│  │             │                      │              │  │
│  │             │         ┌────────────┴────────┐    │  │
│  │             │         │                     │    │  │
│  │             │         ▼                     ▼    │  │
│  │             │    Current Job          Detail     │  │
│  │             │    Panel (if running)   View       │  │
│  │             │                         (modal)    │  │
└──┴─────────────┴─────────────────────────┴──────────┴──┘
```

### Component Hierarchy

1. **App State Manager**: Central state container with methods for:
   - Job data management and caching
   - UI state (selection, filters, sort order)
   - Statistics calculation and caching
   - Progress tracking for running jobs

2. **Layout Manager**: Responsive layout calculation based on terminal size:
   - Determines which components to show
   - Calculates component sizes and positions
   - Handles layout transitions smoothly

3. **Rendering Components**:
   - **Header Panel**: System metrics (CPU, Memory, GPU, Status)
   - **Statistics Dashboard**: Aggregate metrics and trends
   - **Job Table**: Main job list with filtering and sorting
   - **Current Job Panel**: Detailed view of running job
   - **Detail View**: Modal overlay for selected job details
   - **Status Bar**: Keyboard shortcuts and system info

## Components and Interfaces

### App State Structure

```rust
struct App {
    // Core data
    jobs: Vec<Job>,
    system: System,
    
    // UI state
    ui_state: UiState,
    
    // Caching and tracking
    job_progress: HashMap<String, JobProgress>,
    statistics_cache: StatisticsCache,
    estimated_savings_cache: HashMap<String, Option<(f64, f64)>>,
    
    // Configuration
    job_state_dir: PathBuf,
    command_dir: PathBuf,
    
    // Timing
    last_refresh: DateTime<Utc>,
    last_message: Option<String>,
    message_timeout: Option<DateTime<Utc>>,
}

struct UiState {
    // Navigation
    selected_index: Option<usize>,
    scroll_offset: usize,
    
    // Filtering and sorting
    filter: JobFilter,
    sort_mode: SortMode,
    
    // View mode
    view_mode: ViewMode,
    detail_view_job_id: Option<String>,
    
    // Table state
    table_state: TableState,
}

enum JobFilter {
    All,
    Pending,
    Running,
    Success,
    Failed,
}

enum SortMode {
    ByDate,
    BySize,
    ByStatus,
    BySavings,
}

enum ViewMode {
    Normal,
    DetailView,
}
```

### Statistics Cache

```rust
struct StatisticsCache {
    // Aggregate metrics
    total_space_saved: u64,
    average_compression_ratio: f64,
    total_processing_time: i64,
    estimated_pending_savings: u64,
    success_rate: f64,
    
    // Trends (last 20 jobs)
    recent_processing_times: Vec<i64>,
    recent_compression_ratios: Vec<f64>,
    recent_completion_rate: f64,
    
    // Last update time
    last_calculated: DateTime<Utc>,
}

impl StatisticsCache {
    fn calculate(jobs: &[Job]) -> Self;
    fn needs_refresh(&self) -> bool;
}
```

### Enhanced Job Progress Tracking

```rust
struct JobProgress {
    temp_file_path: PathBuf,
    temp_file_size: u64,
    original_size: u64,
    last_updated: DateTime<Utc>,
    bytes_per_second: f64,
    estimated_completion: Option<DateTime<Utc>>,
    stage: JobStage,
    progress_percent: f64,
    
    // New fields for enhanced tracking
    frames_processed: Option<u64>,
    total_frames: Option<u64>,
    current_fps: Option<f64>,
    estimated_final_size: Option<u64>,
    current_compression_ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
enum JobStage {
    Probing,
    Transcoding,
    Verifying,
    Replacing,
    Complete,
}
```

### Color Scheme

```rust
struct ColorScheme {
    // Status colors
    pending: Color,
    running: Color,
    success: Color,
    failed: Color,
    skipped: Color,
    
    // UI element colors
    border_normal: Color,
    border_selected: Color,
    header: Color,
    text_primary: Color,
    text_secondary: Color,
    text_muted: Color,
    
    // Progress colors
    progress_probing: Color,
    progress_transcoding: Color,
    progress_verifying: Color,
    progress_complete: Color,
    
    // Metric colors
    metric_low: Color,
    metric_medium: Color,
    metric_high: Color,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            pending: Color::Yellow,
            running: Color::Green,
            success: Color::Blue,
            failed: Color::Red,
            skipped: Color::Gray,
            
            border_normal: Color::DarkGray,
            border_selected: Color::Cyan,
            header: Color::Cyan,
            text_primary: Color::White,
            text_secondary: Color::Gray,
            text_muted: Color::DarkGray,
            
            progress_probing: Color::Yellow,
            progress_transcoding: Color::Green,
            progress_verifying: Color::Cyan,
            progress_complete: Color::Blue,
            
            metric_low: Color::Green,
            metric_medium: Color::Yellow,
            metric_high: Color::Red,
        }
    }
}
```

## Data Models

### Layout Configuration

```rust
struct LayoutConfig {
    terminal_size: Rect,
    show_statistics: bool,
    show_current_job: bool,
    show_detail_view: bool,
    table_columns: Vec<TableColumn>,
}

enum TableColumn {
    Status,
    File,
    Resolution,
    Codec,
    Bitrate,
    OrigSize,
    NewSize,
    Quality,
    Savings,
    Time,
    Reason,
}

impl LayoutConfig {
    fn from_terminal_size(size: Rect) -> Self {
        let width = size.width;
        let height = size.height;
        
        // Determine what to show based on size
        let show_statistics = height >= 20;
        let table_columns = if width >= 160 {
            // Large terminal: show all columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::Resolution,
                TableColumn::Codec,
                TableColumn::Bitrate,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::Quality,
                TableColumn::Savings,
                TableColumn::Time,
                TableColumn::Reason,
            ]
        } else if width >= 120 {
            // Medium terminal: show essential columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::Resolution,
                TableColumn::Codec,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::Savings,
                TableColumn::Time,
            ]
        } else {
            // Small terminal: minimal columns
            vec![
                TableColumn::Status,
                TableColumn::File,
                TableColumn::OrigSize,
                TableColumn::NewSize,
                TableColumn::Savings,
            ]
        };
        
        Self {
            terminal_size: size,
            show_statistics,
            show_current_job: true,
            show_detail_view: false,
            table_columns,
        }
    }
}
```

### Sparkline Data

```rust
struct SparklineData {
    values: Vec<u64>,
    max_value: u64,
    min_value: u64,
}

impl SparklineData {
    fn from_processing_times(jobs: &[Job]) -> Self;
    fn from_compression_ratios(jobs: &[Job]) -> Self;
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Filter consistency
*For any* job list and filter setting, all jobs displayed in the filtered view should match the filter criteria
**Validates: Requirements 3.3**

### Property 2: Sort order consistency
*For any* job list and sort mode, jobs should be ordered according to the sort criteria (date, size, status, or savings)
**Validates: Requirements 3.5**

### Property 3: Selection bounds
*For any* job list and selection index, the selected index should always be within valid bounds (0 to jobs.len()-1) or None
**Validates: Requirements 3.2**

### Property 4: Statistics accuracy
*For any* set of completed jobs, the calculated total space saved should equal the sum of individual job savings
**Validates: Requirements 5.1**

### Property 5: Progress percentage bounds
*For any* running job with progress tracking, the progress percentage should always be between 0.0 and 100.0 inclusive
**Validates: Requirements 4.1**

### Property 6: Color scheme consistency
*For any* job status, the color used for that job should match the defined color scheme for that status
**Validates: Requirements 1.1**

### Property 7: Layout responsiveness
*For any* terminal size change, the layout should recalculate and render without overlapping components
**Validates: Requirements 1.5, 10.5**

### Property 8: Compression ratio calculation
*For any* completed job with original and new sizes, the compression ratio should equal (original - new) / original
**Validates: Requirements 2.6**

### Property 9: Keyboard shortcut uniqueness
*For any* two different actions, they should be mapped to different keyboard shortcuts
**Validates: Requirements 8.1**

### Property 10: Detail view data completeness
*For any* job displayed in detail view, all available metadata fields should be shown
**Validates: Requirements 6.2, 6.5**

## Error Handling

### Input Validation

- **Invalid terminal size**: Display error message if terminal is too small (< 80x12)
- **Invalid selection**: Clamp selection to valid range or set to None
- **Invalid filter/sort**: Ignore invalid input and maintain current state

### Data Loading Errors

- **Job loading failure**: Display empty table with error message in status bar
- **Missing metadata**: Show "-" or "N/A" for missing fields
- **Corrupted job data**: Skip corrupted jobs and log error

### Rendering Errors

- **Layout calculation failure**: Fall back to minimal layout
- **Component rendering failure**: Skip failed component and continue
- **Color rendering issues**: Fall back to default terminal colors

### Progress Tracking Errors

- **Temp file access failure**: Continue tracking with last known values
- **Invalid progress calculation**: Clamp to valid range (0-100%)
- **ETA calculation overflow**: Display "calculating..." or "-"

## Testing Strategy

### Unit Testing

Unit tests will cover:

1. **Statistics Calculation**:
   - Test total space saved calculation with various job sets
   - Test average compression ratio with edge cases (zero, very large)
   - Test success rate calculation with different status distributions

2. **Filtering Logic**:
   - Test each filter type with mixed job lists
   - Test filter with empty job list
   - Test filter transitions

3. **Sorting Logic**:
   - Test each sort mode with various job orderings
   - Test sort stability (equal elements maintain relative order)
   - Test sort with missing data fields

4. **Layout Calculation**:
   - Test layout for various terminal sizes
   - Test column selection logic
   - Test component visibility logic

5. **Progress Calculation**:
   - Test progress percentage calculation
   - Test ETA calculation with various speeds
   - Test stage detection logic

6. **Color Scheme**:
   - Test status-to-color mapping
   - Test metric-to-color gradient calculation

### Property-Based Testing

Property-based tests will verify:

1. **Filter Consistency** (Property 1):
   - Generate random job lists and filter settings
   - Verify all displayed jobs match filter

2. **Sort Order** (Property 2):
   - Generate random job lists and sort modes
   - Verify jobs are correctly ordered

3. **Selection Bounds** (Property 3):
   - Generate random job lists and selection operations
   - Verify selection is always valid

4. **Statistics Accuracy** (Property 4):
   - Generate random completed jobs with sizes
   - Verify total equals sum of individual savings

5. **Progress Bounds** (Property 5):
   - Generate random progress values
   - Verify all percentages are in [0, 100]

6. **Compression Ratio** (Property 8):
   - Generate random job sizes
   - Verify ratio calculation is correct

### Integration Testing

Integration tests will verify:

1. **End-to-end rendering**: Full UI renders without errors
2. **Input handling**: All keyboard shortcuts work correctly
3. **State transitions**: Filter/sort/view mode changes work correctly
4. **Refresh cycle**: Data refreshes and UI updates correctly

### Manual Testing

Manual testing will focus on:

1. **Visual appearance**: Colors, spacing, alignment look good
2. **Responsiveness**: Layout adapts smoothly to size changes
3. **Usability**: Navigation feels intuitive and responsive
4. **Performance**: UI remains responsive with many jobs
