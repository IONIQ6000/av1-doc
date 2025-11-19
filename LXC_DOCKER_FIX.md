# Fix Docker sysctl Permission Error in LXC Container

## Problem
```
error during container init: open sysctl net.ipv4.ip_unprivileged_port_start file: reopen fd 8: permission denied
```

This is caused by containerd.io 1.7.28-2+ introducing stricter security that conflicts with LXC's AppArmor profiles.

## Solution 1: Downgrade containerd.io (Recommended)

```bash
# Check available versions
apt list -a containerd.io

# Install compatible version (before 1.7.28-2)
apt install containerd.io=1.7.28-1~debian.13~trixie

# Prevent automatic updates
apt-mark hold containerd.io

# Restart services
systemctl restart containerd
systemctl restart docker

# Test
docker run --rm --privileged --device=/dev/dri:/dev/dri lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -version
```

## Solution 2: Modify LXC Container Configuration (Host Side)

If you have access to the LXC host (Proxmox), modify the container config:

```bash
# On the LXC host, edit container config
# For Proxmox: /etc/pve/lxc/<container_id>.conf
# For LXC: /var/lib/lxc/<container_name>/config

# Add these lines:
lxc.apparmor.profile: unconfined
lxc.cgroup2.devices.allow: a
lxc.cap.drop:

# Enable nesting and keyctl (if using Proxmox)
pct set <container_id> -features nesting=1,keyctl=1

# Restart container
pct restart <container_id>
```

## Solution 3: Configure Docker Daemon (Inside LXC Container)

Create/edit `/etc/docker/daemon.json`:

```json
{
  "default-ulimits": {},
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "default-runtime": "runc",
  "runtimes": {
    "runc": {
      "path": "runc"
    }
  }
}
```

Then restart Docker:
```bash
systemctl restart docker
```

## Recommended Approach

**For Debian 13 Trixie in LXC:**
1. Use Solution 1 (downgrade containerd.io) - most reliable
2. If that doesn't work, use Solution 2 (modify LXC config on host)
3. Solution 3 may help but is less likely to fix the root cause

## Verify Fix

After applying a solution, test:

```bash
docker run --rm --privileged --device=/dev/dri:/dev/dri lscr.io/linuxserver/ffmpeg:version-8.0-cli ffmpeg -version
```

If this works, the daemon should work too.

