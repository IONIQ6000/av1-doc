#!/bin/bash
# Script to clear job queue (Pending jobs) and/or history (completed jobs)
# Usage: ./clear-jobs.sh [--queue] [--history] [--all] [--config /path/to/config.json]

set -e

# Default job state directory
JOB_STATE_DIR="/var/lib/av1d/jobs"
CLEAR_QUEUE=false
CLEAR_HISTORY=false
CLEAR_ALL=false
CONFIG_FILE=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --queue)
            CLEAR_QUEUE=true
            shift
            ;;
        --history)
            CLEAR_HISTORY=true
            shift
            ;;
        --all)
            CLEAR_ALL=true
            shift
            ;;
        --config)
            CONFIG_FILE="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --queue       Clear pending jobs (queue)"
            echo "  --history     Clear completed jobs (Success, Failed, Skipped)"
            echo "  --all         Clear all jobs (queue + history + running)"
            echo "  --config PATH Path to config.json (default: /etc/av1d/config.json)"
            echo "  -h, --help    Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0 --queue              # Clear only pending jobs"
            echo "  $0 --history            # Clear only completed jobs"
            echo "  $0 --queue --history    # Clear both queue and history"
            echo "  $0 --all                # Clear everything"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# If no options specified, show help
if [[ "$CLEAR_QUEUE" == false && "$CLEAR_HISTORY" == false && "$CLEAR_ALL" == false ]]; then
    echo "Error: No action specified"
    echo "Use --help for usage information"
    exit 1
fi

# Load job_state_dir from config if available
if [[ -z "$CONFIG_FILE" ]]; then
    CONFIG_FILE="/etc/av1d/config.json"
fi

if [[ -f "$CONFIG_FILE" ]]; then
    # Try to extract job_state_dir from config (requires jq)
    if command -v jq &> /dev/null; then
        JOB_STATE_DIR=$(jq -r '.job_state_dir // "/var/lib/av1d/jobs"' "$CONFIG_FILE")
    fi
fi

# Verify job state directory exists
if [[ ! -d "$JOB_STATE_DIR" ]]; then
    echo "Error: Job state directory does not exist: $JOB_STATE_DIR"
    exit 1
fi

echo "Job state directory: $JOB_STATE_DIR"
echo ""

# Function to get status from a job file
get_job_status() {
    local job_file="$1"
    if [[ -f "$job_file" ]] && command -v jq &> /dev/null; then
        jq -r '.status' "$job_file" 2>/dev/null || echo "unknown"
    else
        echo "unknown"
    fi
}

# Count jobs by status
pending_count=0
running_count=0
success_count=0
failed_count=0
skipped_count=0
unknown_count=0

for job_file in "$JOB_STATE_DIR"/*.json; do
    if [[ -f "$job_file" ]]; then
        status=$(get_job_status "$job_file")
        case "$status" in
            pending) ((pending_count++)) ;;
            running) ((running_count++)) ;;
            success) ((success_count++)) ;;
            failed) ((failed_count++)) ;;
            skipped) ((skipped_count++)) ;;
            *) ((unknown_count++)) ;;
        esac
    fi
done

total_jobs=$((pending_count + running_count + success_count + failed_count + skipped_count + unknown_count))

echo "Current job counts:"
echo "  Pending: $pending_count"
echo "  Running: $running_count"
echo "  Success: $success_count"
echo "  Failed: $failed_count"
echo "  Skipped: $skipped_count"
if [[ $unknown_count -gt 0 ]]; then
    echo "  Unknown: $unknown_count"
fi
echo "  Total: $total_jobs"
echo ""

# Warn if trying to clear running jobs
if [[ "$CLEAR_ALL" == true && $running_count -gt 0 ]]; then
    echo "⚠️  WARNING: There are $running_count running job(s)."
    echo "   Clearing running jobs may cause issues with active transcoding."
    read -p "   Continue anyway? (yes/no): " confirm
    if [[ "$confirm" != "yes" ]]; then
        echo "Aborted."
        exit 0
    fi
fi

# Clear jobs
deleted_count=0

for job_file in "$JOB_STATE_DIR"/*.json; do
    if [[ -f "$job_file" ]]; then
        status=$(get_job_status "$job_file")
        should_delete=false
        
        if [[ "$CLEAR_ALL" == true ]]; then
            should_delete=true
        elif [[ "$CLEAR_QUEUE" == true && "$status" == "pending" ]]; then
            should_delete=true
        elif [[ "$CLEAR_HISTORY" == true && ("$status" == "success" || "$status" == "failed" || "$status" == "skipped") ]]; then
            should_delete=true
        fi
        
        if [[ "$should_delete" == true ]]; then
            rm -f "$job_file"
            ((deleted_count++))
        fi
    fi
done

echo "✅ Deleted $deleted_count job file(s)"

# Show remaining counts
if [[ $deleted_count -gt 0 ]]; then
    echo ""
    echo "Remaining jobs:"
    remaining_pending=0
    remaining_running=0
    remaining_completed=0
    
    for job_file in "$JOB_STATE_DIR"/*.json; do
        if [[ -f "$job_file" ]]; then
            status=$(get_job_status "$job_file")
            case "$status" in
                pending) ((remaining_pending++)) ;;
                running) ((remaining_running++)) ;;
                success|failed|skipped) ((remaining_completed++)) ;;
            esac
        fi
    done
    
    echo "  Pending: $remaining_pending"
    echo "  Running: $remaining_running"
    echo "  Completed: $remaining_completed"
fi

