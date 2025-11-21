# Tilde Expansion Support Added

## What Changed

Added automatic tilde (`~`) expansion for all paths in the configuration file.

## Now You Can Use

```json
{
  "temp_output_dir": "~/temp",
  "job_state_dir": "~/av1d/jobs",
  "library_roots": ["~/media", "/mnt/storage"]
}
```

The `~` will automatically expand to your home directory (e.g., `/root` or `/home/username`).

## Implementation

### File: `crates/daemon/src/config.rs`

Added `expand_tilde()` function:
```rust
fn expand_tilde(path: &Path) -> PathBuf {
    if let Some(path_str) = path.to_str() {
        if path_str.starts_with("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                let home_path = PathBuf::from(home);
                return home_path.join(&path_str[2..]); // Skip "~/"
            }
        } else if path_str == "~" {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home);
            }
        }
    }
    path.to_path_buf()
}
```

Called automatically after loading config:
```rust
pub fn load_config(path: Option<&Path>) -> Result<Self> {
    // ... load config from file ...
    
    // Expand tilde (~) in paths after loading
    config.expand_tilde_in_paths();
    
    Ok(config)
}
```

## Supported Paths

Tilde expansion works for:
- `temp_output_dir`
- `job_state_dir`
- `library_roots` (all entries)
- `docker_bin`
- `gpu_device`
- `command_dir`

## Examples

| Config Value | Expands To (if HOME=/root) |
|--------------|----------------------------|
| `~/temp` | `/root/temp` |
| `~` | `/root` |
| `~/media/movies` | `/root/media/movies` |
| `/absolute/path` | `/absolute/path` (unchanged) |

## Your Config Now Works

Your original config:
```json
"temp_output_dir":"~/temp"
```

Will now correctly expand to `/root/temp` (or whatever your `$HOME` is).

## Testing

All 13 tests pass:
```
test result: ok. 13 passed; 0 failed; 0 ignored
```

## Deployment

Just rebuild and deploy:
```bash
cargo build --release
scp target/release/av1d root@container:/usr/local/bin/
systemctl restart av1d
```

Your `~/temp` config will now work correctly!
