#!/bin/bash
# Test script to verify FFmpeg 8.0 av1_vaapi encoder capabilities

set -e

DOCKER_IMAGE="lscr.io/linuxserver/ffmpeg:version-8.0-cli"
OUTPUT_DIR="./ffmpeg_test_results"

echo "=========================================="
echo "FFmpeg 8.0 av1_vaapi Capability Test"
echo "=========================================="
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Test 1: Get encoder help
echo "Test 1: Checking av1_vaapi encoder parameters..."
docker run --rm "$DOCKER_IMAGE" \
  ffmpeg -h encoder=av1_vaapi > "$OUTPUT_DIR/av1_vaapi_help.txt" 2>&1

echo "✓ Saved to: $OUTPUT_DIR/av1_vaapi_help.txt"
echo ""

# Display key parameters
echo "Key parameters found:"
echo "---"
grep -E "(-qp|-quality|-profile|-rc_mode|-tier|-tile)" "$OUTPUT_DIR/av1_vaapi_help.txt" || echo "  (searching...)"
echo "---"
echo ""

# Test 2: Check FFmpeg version
echo "Test 2: Checking FFmpeg version..."
docker run --rm "$DOCKER_IMAGE" \
  ffmpeg -version > "$OUTPUT_DIR/ffmpeg_version.txt" 2>&1

echo "✓ Saved to: $OUTPUT_DIR/ffmpeg_version.txt"
head -n 3 "$OUTPUT_DIR/ffmpeg_version.txt"
echo ""

# Test 3: Check VAAPI support
echo "Test 3: Checking VAAPI encoders available..."
docker run --rm "$DOCKER_IMAGE" \
  ffmpeg -encoders 2>&1 | grep -i "av1.*vaapi" > "$OUTPUT_DIR/vaapi_encoders.txt" || true

if [ -s "$OUTPUT_DIR/vaapi_encoders.txt" ]; then
    echo "✓ AV1 VAAPI encoder found:"
    cat "$OUTPUT_DIR/vaapi_encoders.txt"
else
    echo "⚠ No AV1 VAAPI encoder found in output"
fi
echo ""

# Test 4: Check pixel format support
echo "Test 4: Checking supported pixel formats..."
docker run --rm "$DOCKER_IMAGE" \
  ffmpeg -pix_fmts 2>&1 | grep -E "(nv12|p010)" > "$OUTPUT_DIR/pixel_formats.txt" || true

echo "✓ Saved to: $OUTPUT_DIR/pixel_formats.txt"
echo "Key formats:"
grep -E "(nv12|p010le)" "$OUTPUT_DIR/pixel_formats.txt" || echo "  (checking file...)"
echo ""

# Test 5: Parse encoder options
echo "Test 5: Analyzing encoder options..."
echo ""

# Check for specific parameters
echo "Parameter Support Analysis:"
echo "---"

if grep -q "\-qp" "$OUTPUT_DIR/av1_vaapi_help.txt"; then
    echo "✓ -qp parameter: SUPPORTED"
    grep "\-qp" "$OUTPUT_DIR/av1_vaapi_help.txt" | head -n 1
else
    echo "✗ -qp parameter: NOT FOUND"
fi

if grep -q "\-quality" "$OUTPUT_DIR/av1_vaapi_help.txt"; then
    echo "✓ -quality parameter: SUPPORTED"
    grep "\-quality" "$OUTPUT_DIR/av1_vaapi_help.txt" | head -n 1
else
    echo "✗ -quality parameter: NOT FOUND"
fi

if grep -q "\-profile" "$OUTPUT_DIR/av1_vaapi_help.txt"; then
    echo "✓ -profile parameter: SUPPORTED"
    grep "\-profile" "$OUTPUT_DIR/av1_vaapi_help.txt" | head -n 1
else
    echo "✗ -profile parameter: NOT FOUND"
fi

if grep -q "\-rc_mode" "$OUTPUT_DIR/av1_vaapi_help.txt"; then
    echo "✓ -rc_mode parameter: SUPPORTED"
    grep "\-rc_mode" "$OUTPUT_DIR/av1_vaapi_help.txt" | head -n 1
else
    echo "✗ -rc_mode parameter: NOT FOUND"
fi

if grep -q "\-tile" "$OUTPUT_DIR/av1_vaapi_help.txt"; then
    echo "✓ -tile parameters: SUPPORTED"
    grep "\-tile" "$OUTPUT_DIR/av1_vaapi_help.txt" | head -n 2
else
    echo "✗ -tile parameters: NOT FOUND"
fi

echo "---"
echo ""

# Summary
echo "=========================================="
echo "Summary"
echo "=========================================="
echo ""
echo "Results saved to: $OUTPUT_DIR/"
echo ""
echo "Files created:"
echo "  - av1_vaapi_help.txt (full encoder help)"
echo "  - ffmpeg_version.txt (FFmpeg version info)"
echo "  - vaapi_encoders.txt (available VAAPI encoders)"
echo "  - pixel_formats.txt (supported pixel formats)"
echo ""
echo "Next steps:"
echo "  1. Review $OUTPUT_DIR/av1_vaapi_help.txt for full parameter list"
echo "  2. If you have test video files, run encoding tests"
echo "  3. Update implementation plan based on supported parameters"
echo ""
echo "To test actual encoding (requires test file and GPU access):"
echo "  See FFMPEG_8_COMPATIBILITY_CHECK.md for test commands"
echo ""
