#!/bin/bash
# Script to create test sample video files for QSV integration testing

echo "Creating test sample video files..."
echo ""

# Check if ffmpeg is available
if ! command -v ffmpeg &> /dev/null; then
    echo "ERROR: ffmpeg not found. Please install ffmpeg to create sample files."
    echo ""
    echo "Alternatives:"
    echo "1. Use Docker to create samples:"
    echo "   docker run --rm -v \$(pwd):/data lscr.io/linuxserver/ffmpeg:version-8.0-cli \\"
    echo "     -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \\"
    echo "     -c:v libx264 -pix_fmt yuv420p /data/sample_8bit_h264.mp4"
    echo ""
    echo "2. Download sample files from:"
    echo "   - https://kodi.wiki/view/Samples"
    echo "   - https://github.com/jellyfin/jellyfin-test-media"
    exit 1
fi

# Create 8-bit H.264 sample
echo "Creating 8-bit H.264 sample (sample_8bit_h264.mp4)..."
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx264 -pix_fmt yuv420p -preset fast \
  -y sample_8bit_h264.mp4 2>&1 | grep -E "frame=|time=|size=" | tail -1

if [ -f "sample_8bit_h264.mp4" ]; then
    echo "✓ Created sample_8bit_h264.mp4"
    ls -lh sample_8bit_h264.mp4
else
    echo "✗ Failed to create 8-bit sample"
fi
echo ""

# Create 10-bit HEVC sample
echo "Creating 10-bit HEVC sample (sample_10bit_hevc.mkv)..."
ffmpeg -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx265 -pix_fmt yuv420p10le -preset fast \
  -y sample_10bit_hevc.mkv 2>&1 | grep -E "frame=|time=|size=" | tail -1

if [ -f "sample_10bit_hevc.mkv" ]; then
    echo "✓ Created sample_10bit_hevc.mkv"
    ls -lh sample_10bit_hevc.mkv
else
    echo "✗ Failed to create 10-bit sample"
fi
echo ""

echo "Sample files created successfully!"
echo ""
echo "You can now run the integration tests:"
echo "  ./integration_test_qsv.sh sample_8bit_h264.mp4 sample_10bit_hevc.mkv"
