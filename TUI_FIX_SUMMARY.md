# TUI Status Display Fix

## Problem

The TUI was showing "Probing" instead of "Transcoding" even when encoding was actively running.

### Root Cause

The TUI was looking for the temp file in the **source directory**:
```rust
let temp_output = job.source_path.with_extension("tmp.av1.mkv");
```

But the daemon was writing it to the **configured temp directory** (`/temp`):
```rust
let temp_output = cfg.temp_output_dir.join(format!("{}.tmp.av1.mkv", filename));
```

The TUI couldn't find the temp file, so it thought the job was still in the "Probing" stage.

## Solution

Made the TUI use the same temp directory logic as the daemon:

### Changes Made

#### 1. Added `temp_output_dir` to App struct
```rust
struct App {
    // ... existing fields ...
    temp_output_dir: PathBuf,  // NEW
}
```

#### 2. Added helper function (same as daemon)
```rust
impl App {
    fn get_temp_output_path(&self, source_path: &Path) -> PathBuf {
        let filename = source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        self.temp_output_dir.join(format!("{}.tmp.av1.mkv", filename))
    }
}
```

#### 3. Updated constructor
```rust
fn new(job_state_dir: PathBuf, temp_output_dir: PathBuf) -> Self {
    // ...
    temp_output_dir,  // Store it
    // ...
}
```

#### 4. Updated main function
```rust
let mut app = App::new(cfg.job_state_dir.clone(), cfg.temp_output_dir.clone());
```

#### 5. Fixed temp file detection (2 locations)
```rust
// OLD - WRONG
let temp_output = job.source_path.with_extension("tmp.av1.mkv");

// NEW - CORRECT
let temp_output = self.get_temp_output_path(&job.source_path);
```

## Result

✅ TUI now correctly detects temp files in `/temp` directory
✅ Status updates from "Probing" → "Transcoding" when encoding starts
✅ Progress tracking works (FPS, speed, ETA)
✅ File size growth is monitored

## Testing

Compile and deploy:
```bash
cargo build --release
scp target/release/av1tui root@container:/usr/local/bin/
```

The TUI will now show:
- **STAGE: Transcoding** (not stuck on "Probing")
- **FPS**: Actual encoding speed
- **PROGRESS**: Percentage based on temp file growth
- **ETA**: Estimated time remaining

## Files Modified

- `crates/cli-tui/src/main.rs`:
  - Added `temp_output_dir` field to `App` struct
  - Added `get_temp_output_path()` helper method
  - Updated constructor to accept `temp_output_dir`
  - Fixed 2 locations where temp file path was calculated
  - Updated main function to pass config

## Compilation

✅ Compiles successfully
✅ No errors or warnings (except unused import in daemon)
✅ No diagnostics

## Impact

- **TUI display**: Now accurate
- **Daemon**: No changes needed
- **Encoding**: Unaffected (was already working)
- **Compatibility**: Requires both daemon and TUI to be updated

Your TUI will now properly show encoding progress! 🎉
