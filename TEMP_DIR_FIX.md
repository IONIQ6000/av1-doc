# Temp Directory Fix - No More Fallback

## Problem

The daemon was ignoring the `temp_output_dir` configuration and writing temporary files to the source video's directory (spinning disk) instead of the configured fast storage (NVMe).

### Root Cause

The Docker command was mounting only the **input file's parent directory** as `/config`, and trying to write both input and output to the same location:

```rust
// OLD CODE - WRONG
.arg(format!("{}:/config", parent_dir.display()))  // Only input dir mounted
let container_input = format!("/config/{}", input_basename);
let container_output = format!("/config/{}", output_basename);  // Same dir!
```

This completely ignored the `temp_output_dir` configuration.

## Solution

Mount **input and output directories separately** in Docker:

```rust
// NEW CODE - CORRECT
.arg(format!("{}:/input:ro", input_parent_dir.display()))    // Read-only input
.arg(format!("{}:/output", output_parent_dir.display()))     // Writable output (temp dir)

let container_input = format!("/input/{}", input_basename);
let container_output = format!("/output/{}", output_basename);
```

## Changes Made

### File: `crates/daemon/src/ffmpeg_docker.rs`

1. **Separate parent directories** for input and output:
   ```rust
   let input_parent_dir = input.parent()?;
   let output_parent_dir = temp_output.parent()?;
   ```

2. **Mount both directories** in Docker:
   ```rust
   .arg("-v")
   .arg(format!("{}:/input:ro", input_parent_dir.display()))
   .arg("-v")
   .arg(format!("{}:/output", output_parent_dir.display()))
   ```

3. **Updated container paths**:
   ```rust
   let container_input = format!("/input/{}", input_basename);
   let container_output = format!("/output/{}", output_basename);
   ```

4. **Updated debug logging** to show both mounts

## Benefits

✅ **No fallback** - Output ALWAYS goes to `temp_output_dir`
✅ **Fast storage** - Temp files written to NVMe, not spinning disk
✅ **Read-only input** - Source files protected from accidental modification
✅ **Explicit separation** - Clear distinction between source and temp storage

## Testing

All 13 property-based tests pass:
```
test result: ok. 13 passed; 0 failed; 0 ignored
```

## Deployment

1. **Rebuild the daemon**:
   ```bash
   cargo build --release
   ```

2. **Copy to your Debian container**:
   ```bash
   scp target/release/av1d user@container:/usr/local/bin/
   ```

3. **Fix your config** (change `~/temp` to absolute path):
   ```bash
   nano /etc/av1d/config.json
   # Change: "temp_output_dir":"~/temp"
   # To:     "temp_output_dir":"/root/temp"  (or your NVMe path)
   ```

4. **Create temp directory**:
   ```bash
   mkdir -p /root/temp
   # OR
   mkdir -p /nvme/av1-temp
   ```

5. **Restart daemon**:
   ```bash
   systemctl restart av1d
   ```

6. **Verify it's working**:
   ```bash
   journalctl -u av1d -f | grep "temp directory"
   ```

You should see:
```
Job X: Using fast temp directory: /root/temp
Job X: Temp output will be: /root/temp/movie.tmp.av1.mkv
```

And in Docker logs:
```
🎬 QSV encoding: docker run ... -v /media/movies:/input:ro -v /root/temp:/output ...
```

## Important Notes

### Config File Issue

Your config had:
```json
"temp_output_dir":"~/temp"
```

The `~` (tilde) doesn't expand in JSON files. You MUST use an absolute path:
```json
"temp_output_dir":"/root/temp"
```

### No More Fallback

With this fix, if `temp_output_dir` doesn't exist or isn't writable, the job will **fail immediately** instead of silently falling back to the source directory. This is intentional - it forces you to fix the configuration.

## Verification

After deploying, check that temp files appear in your configured directory:

```bash
# Watch temp directory during encoding
watch -n 1 ls -lh /root/temp/

# Should see .tmp.av1.mkv files appear during encoding
```

Your spinning disks should now be completely idle during encoding!
