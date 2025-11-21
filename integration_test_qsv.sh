#!/bin/bash
# Integration Test Script for AV1 QSV Migration
# This script helps verify the QSV implementation with real hardware

set -e

echo "==================================="
echo "AV1 QSV Integration Test Suite"
echo "==================================="
echo ""

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check prerequisites
echo "Checking prerequisites..."

if ! command -v docker &> /dev/null; then
    echo -e "${RED}ERROR: Docker is not installed${NC}"
    exit 1
fi

if ! command -v ffprobe &> /dev/null; then
    echo -e "${YELLOW}WARNING: ffprobe not found locally, will use Docker${NC}"
fi

if [ ! -d "/dev/dri" ]; then
    echo -e "${RED}ERROR: /dev/dri not found - GPU device not available${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Prerequisites check passed${NC}"
echo ""

# Test 1: Verify Docker image
echo "Test 1: Verifying Docker image..."
if docker pull lscr.io/linuxserver/ffmpeg:version-8.0-cli; then
    echo -e "${GREEN}✓ Docker image available${NC}"
else
    echo -e "${RED}✗ Failed to pull Docker image${NC}"
    exit 1
fi
echo ""

# Test 2: Verify QSV support in Docker image
echo "Test 2: Verifying QSV support..."
if docker run --rm --privileged \
    --device /dev/dri:/dev/dri \
    -e LIBVA_DRIVER_NAME=iHD \
    lscr.io/linuxserver/ffmpeg:version-8.0-cli \
    -hide_banner -encoders | grep -q "av1_qsv"; then
    echo -e "${GREEN}✓ av1_qsv encoder available${NC}"
else
    echo -e "${RED}✗ av1_qsv encoder not found${NC}"
    exit 1
fi
echo ""

# Function to analyze video file
analyze_video() {
    local file=$1
    echo "Analyzing: $file"
    
    if command -v ffprobe &> /dev/null; then
        ffprobe -v error -select_streams v:0 \
            -show_entries stream=codec_name,pix_fmt,width,height,bit_depth \
            -of default=noprint_wrappers=1 "$file"
    else
        docker run --rm -v "$(pwd):/data" \
            lscr.io/linuxserver/ffmpeg:version-8.0-cli \
            -v error -select_streams v:0 \
            -show_entries stream=codec_name,pix_fmt,width,height,bit_depth \
            -of default=noprint_wrappers=1 "/data/$file"
    fi
}

# Function to test encoding
test_encode() {
    local input=$1
    local output=$2
    local expected_pix_fmt=$3
    local test_name=$4
    
    echo "========================================="
    echo "Test: $test_name"
    echo "========================================="
    echo "Input: $input"
    echo "Output: $output"
    echo "Expected pixel format: $expected_pix_fmt"
    echo ""
    
    if [ ! -f "$input" ]; then
        echo -e "${RED}✗ Input file not found: $input${NC}"
        echo -e "${YELLOW}Please provide a sample file${NC}"
        return 1
    fi
    
    echo "Source file analysis:"
    analyze_video "$input"
    echo ""
    
    echo "Starting encoding..."
    local start_time=$(date +%s)
    
    # Run encoding with QSV
    if docker run --rm --privileged \
        --user root \
        --device /dev/dri:/dev/dri \
        -e LIBVA_DRIVER_NAME=iHD \
        -v "$(pwd):/data" \
        lscr.io/linuxserver/ffmpeg:version-8.0-cli \
        -init_hw_device qsv=hw:/dev/dri/renderD128 \
        -filter_hw_device hw \
        -i "/data/$input" \
        -vf "format=${expected_pix_fmt},hwupload" \
        -c:v av1_qsv \
        -global_quality 30 \
        -profile:v main \
        -c:a copy \
        -y "/data/$output" 2>&1 | tee encode_log.txt; then
        
        local end_time=$(date +%s)
        local duration=$((end_time - start_time))
        
        echo ""
        echo -e "${GREEN}✓ Encoding completed in ${duration}s${NC}"
        
        # Verify output
        echo ""
        echo "Output file analysis:"
        analyze_video "$output"
        
        # Check pixel format
        local actual_pix_fmt
        if command -v ffprobe &> /dev/null; then
            actual_pix_fmt=$(ffprobe -v error -select_streams v:0 \
                -show_entries stream=pix_fmt \
                -of default=noprint_wrappers=1:nokey=1 "$output")
        else
            actual_pix_fmt=$(docker run --rm -v "$(pwd):/data" \
                lscr.io/linuxserver/ffmpeg:version-8.0-cli \
                -v error -select_streams v:0 \
                -show_entries stream=pix_fmt \
                -of default=noprint_wrappers=1:nokey=1 "/data/$output")
        fi
        
        echo ""
        if [ "$actual_pix_fmt" = "$expected_pix_fmt" ]; then
            echo -e "${GREEN}✓ Pixel format correct: $actual_pix_fmt${NC}"
        else
            echo -e "${RED}✗ Pixel format mismatch!${NC}"
            echo "  Expected: $expected_pix_fmt"
            echo "  Actual: $actual_pix_fmt"
            return 1
        fi
        
        # Extract encoding stats from log
        echo ""
        echo "Encoding statistics:"
        grep -E "frame=|fps=|speed=" encode_log.txt | tail -1 || echo "Stats not available"
        
        return 0
    else
        echo -e "${RED}✗ Encoding failed${NC}"
        return 1
    fi
}

# Main test execution
echo "========================================="
echo "INTEGRATION TEST EXECUTION"
echo "========================================="
echo ""
echo "This script will test AV1 QSV encoding with:"
echo "1. 8-bit H.264 source → 8-bit AV1 (yuv420p)"
echo "2. 10-bit HEVC source → 10-bit AV1 (yuv420p10le)"
echo ""
echo "Please ensure you have sample files available:"
echo "  - sample_8bit_h264.mp4 (or similar)"
echo "  - sample_10bit_hevc.mkv (or similar)"
echo ""

# Check for sample files
if [ -n "$1" ]; then
    INPUT_8BIT="$1"
else
    INPUT_8BIT="sample_8bit_h264.mp4"
fi

if [ -n "$2" ]; then
    INPUT_10BIT="$2"
else
    INPUT_10BIT="sample_10bit_hevc.mkv"
fi

echo "Using sample files:"
echo "  8-bit: $INPUT_8BIT"
echo "  10-bit: $INPUT_10BIT"
echo ""

# Test 3: 8-bit encoding
if test_encode "$INPUT_8BIT" "output_8bit_qsv.mkv" "yuv420p" "8-bit H.264 → AV1 QSV"; then
    echo -e "${GREEN}✓ Test 3 PASSED: 8-bit encoding${NC}"
else
    echo -e "${RED}✗ Test 3 FAILED: 8-bit encoding${NC}"
fi
echo ""

# Test 4: 10-bit encoding
if test_encode "$INPUT_10BIT" "output_10bit_qsv.mkv" "yuv420p10le" "10-bit HEVC → AV1 QSV"; then
    echo -e "${GREEN}✓ Test 4 PASSED: 10-bit encoding${NC}"
else
    echo -e "${RED}✗ Test 4 FAILED: 10-bit encoding${NC}"
fi
echo ""

# GPU utilization check
echo "========================================="
echo "GPU Utilization Check"
echo "========================================="
echo ""
echo "To monitor GPU utilization during encoding, run in another terminal:"
echo "  watch -n 1 'cat /sys/class/drm/card*/device/gpu_busy_percent'"
echo ""
echo "Or use intel_gpu_top if available:"
echo "  sudo intel_gpu_top"
echo ""

echo "========================================="
echo "Integration Test Complete"
echo "========================================="
echo ""
echo "Manual verification checklist:"
echo "  [ ] Both encodings completed successfully"
echo "  [ ] Output pixel formats are correct"
echo "  [ ] Encoding speed is acceptable (check fps in logs)"
echo "  [ ] GPU utilization was observed during encoding"
echo "  [ ] Output files play correctly"
echo ""
echo "Next steps:"
echo "  1. Review encode_log.txt for detailed ffmpeg output"
echo "  2. Play output files to verify quality"
echo "  3. Compare encoding speed with previous VAAPI implementation"
echo "  4. Update task status in tasks.md"
