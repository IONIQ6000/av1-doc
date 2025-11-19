# Docker Troubleshooting Guide

## Permission Denied Error with sysctl

If you see errors like:
```
error during container init: open sysctl net.ipv4.ip_unprivileged_port_start file: permission denied
```

This typically happens when Docker is running inside an LXC container or with restricted permissions.

### Solution 1: Configure Docker Daemon (Recommended)

Edit `/etc/docker/daemon.json`:

```json
{
  "default-ulimits": {},
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  }
}
```

Then restart Docker:
```bash
systemctl restart docker
```

### Solution 2: Use --privileged Flag (Less Secure)

If Solution 1 doesn't work, you can modify the Docker commands to use `--privileged`:

**Note:** This gives the container full host access and is less secure. Only use if necessary.

### Solution 3: Configure LXC Container

If running Docker inside an LXC container, you may need to configure the LXC container to allow sysctl access:

```bash
# Edit LXC config
nano /var/lib/lxc/<container-name>/config

# Add:
lxc.apparmor.profile = unconfined
lxc.cap.drop =
```

### Solution 4: Use Different Docker Image

If the above solutions don't work, consider using a different ffmpeg Docker image that doesn't require sysctl modifications.

## Testing Docker

Test if Docker works with the image:

```bash
# Basic test
docker run --rm lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -version

# With device passthrough
docker run --rm --device=/dev/dri:/dev/dri lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -version

# With security opts (current implementation)
docker run --rm --security-opt seccomp=unconfined --device=/dev/dri:/dev/dri lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -version
```

## Current Implementation

The code currently uses `--security-opt seccomp=unconfined` to bypass sysctl permission issues. This is a reasonable compromise between security and functionality.

