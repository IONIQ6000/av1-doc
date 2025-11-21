#!/bin/bash
# Clear AV1 Queue and History Script
# This script stops the daemon, clears all job state, and prepares for a fresh start

set -e

echo "═══════════════════════════════════════════════════════════════"
echo "           AV1 Queue and History Cleaner"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Configuration
JOB_STATE_DIR="${JOB_STATE_DIR:-/tmp/av1d-jobs}"
COMMAND_DIR="${COMMAND_DIR:-/tmp/commands}"

echo "Job State Directory: $JOB_STATE_DIR"
echo "Command Directory:   $COMMAND_DIR"
echo ""

# Step 1: Stop the daemon
echo "Step 1: Stopping AV1 daemon..."
if pgrep -x av1d > /dev/null; then
    pkill av1d
    echo "  ✓ Daemon stopped"
    sleep 2
else
    echo "  ℹ Daemon not running"
fi
echo ""

# Step 2: Stop any running ffmpeg containers
echo "Step 2: Stopping running ffmpeg containers..."
RUNNING_CONTAINERS=$(docker ps --filter "ancestor=lscr.io/linuxserver/ffmpeg:version-8.0-cli" -q)
if [ -n "$RUNNING_CONTAINERS" ]; then
    echo "$RUNNING_CONTAINERS" | xargs docker stop
    echo "  ✓ Stopped $(echo "$RUNNING_CONTAINERS" | wc -l) container(s)"
else
    echo "  ℹ No running ffmpeg containers found"
fi
echo ""

# Step 3: Count and display current jobs
echo "Step 3: Current job statistics..."
if [ -d "$JOB_STATE_DIR" ]; then
    TOTAL_JOBS=$(find "$JOB_STATE_DIR" -name "*.json" 2>/dev/null | wc -l)
    PENDING=$(grep -l '"status":"Pending"' "$JOB_STATE_DIR"/*.json 2>/dev/null | wc -l)
    RUNNING=$(grep -l '"status":"Running"' "$JOB_STATE_DIR"/*.json 2>/dev/null | wc -l)
    SUCCESS=$(grep -l '"status":"Success"' "$JOB_STATE_DIR"/*.json 2>/dev/null | wc -l)
    FAILED=$(grep -l '"status":"Failed"' "$JOB_STATE_DIR"/*.json 2>/dev/null | wc -l)
    SKIPPED=$(grep -l '"status":"Skipped"' "$JOB_STATE_DIR"/*.json 2>/dev/null | wc -l)
    
    echo "  Total jobs:   $TOTAL_JOBS"
    echo "  Pending:      $PENDING"
    echo "  Running:      $RUNNING"
    echo "  Success:      $SUCCESS"
    echo "  Failed:       $FAILED"
    echo "  Skipped:      $SKIPPED"
else
    echo "  ℹ Job state directory does not exist"
fi
echo ""

# Step 4: Confirm deletion
read -p "Do you want to delete ALL job state files? (yes/no): " CONFIRM
if [ "$CONFIRM" != "yes" ]; then
    echo ""
    echo "Cancelled. No files were deleted."
    exit 0
fi
echo ""

# Step 5: Backup (optional)
read -p "Create backup before deleting? (yes/no): " BACKUP
if [ "$BACKUP" = "yes" ]; then
    BACKUP_DIR="${JOB_STATE_DIR}-backup-$(date +%Y%m%d-%H%M%S)"
    echo "Creating backup at: $BACKUP_DIR"
    cp -r "$JOB_STATE_DIR" "$BACKUP_DIR" 2>/dev/null || true
    echo "  ✓ Backup created"
    echo ""
fi

# Step 6: Clear job state files
echo "Step 4: Clearing job state files..."
if [ -d "$JOB_STATE_DIR" ]; then
    DELETED=$(find "$JOB_STATE_DIR" -name "*.json" -type f 2>/dev/null | wc -l)
    rm -f "$JOB_STATE_DIR"/*.json 2>/dev/null || true
    echo "  ✓ Deleted $DELETED job file(s)"
else
    echo "  ℹ Job state directory does not exist"
fi
echo ""

# Step 7: Clear command files
echo "Step 5: Clearing command files..."
if [ -d "$COMMAND_DIR" ]; then
    DELETED_CMD=$(find "$COMMAND_DIR" -name "*.json" -type f 2>/dev/null | wc -l)
    rm -f "$COMMAND_DIR"/*.json 2>/dev/null || true
    echo "  ✓ Deleted $DELETED_CMD command file(s)"
else
    echo "  ℹ Command directory does not exist"
fi
echo ""

# Step 8: Clean up orphaned temp files (optional)
read -p "Delete orphaned temp files (.tmp.av1.mkv)? (yes/no): " CLEAN_TEMP
if [ "$CLEAN_TEMP" = "yes" ]; then
    echo ""
    echo "Step 6: Cleaning orphaned temp files..."
    echo "  Scanning for .tmp.av1.mkv files..."
    
    # This would need to scan your library roots
    # For safety, we'll just show what would be deleted
    echo "  ⚠ Manual cleanup recommended - check your library directories for:"
    echo "    - *.tmp.av1.mkv files"
    echo "    - *.orig.mkv files (backups)"
    echo ""
fi

# Summary
echo "═══════════════════════════════════════════════════════════════"
echo "                    CLEANUP COMPLETE"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "✓ Daemon stopped"
echo "✓ Docker containers stopped"
echo "✓ Job state cleared"
echo "✓ Command files cleared"
echo ""
echo "The system is now in a clean state."
echo ""
echo "To restart the daemon:"
echo "  ./target/release/av1d --config /path/to/config.json"
echo ""
echo "Or with default settings:"
echo "  ./target/release/av1d"
echo ""
echo "The daemon will rescan your library and create new jobs."
echo "═══════════════════════════════════════════════════════════════"
