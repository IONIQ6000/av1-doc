#!/bin/bash
# Script to create test sample video files using Docker
# This works even if ffmpeg is not installed locally

echo "==================================="
echo "Creating Test Samples with Docker"
echo "==================================="
echo ""

# Create 8-bit H.264 sample
echo "Creating 8-bit H.264 sample (10 seconds, 1080p)..."
docker run --rm -v "$(pwd):/data" \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx264 -pix_fmt yuv420p -preset fast \
  -y /data/sample_8bit_h264.mp4

if [ -f "sample_8bit_h264.mp4" ]; then
    echo "✓ Created sample_8bit_h264.mp4"
    ls -lh sample_8bit_h264.mp4
    
    # Verify with ffprobe
    echo "  Verifying..."
    docker run --rm -v "$(pwd):/data" \
      --entrypoint ffprobe \
      lscr.io/linuxserver/ffmpeg:version-8.0-cli \
      -v error -select_streams v:0 \
      -show_entries stream=codec_name,pix_fmt,bit_depth \
      -of default=noprint_wrappers=1 /data/sample_8bit_h264.mp4
else
    echo "✗ Failed to create 8-bit sample"
fi
echo ""

# Create 10-bit HEVC sample
echo "Creating 10-bit HEVC sample (10 seconds, 1080p)..."
docker run --rm -v "$(pwd):/data" \
  lscr.io/linuxserver/ffmpeg:version-8.0-cli \
  -f lavfi -i testsrc=duration=10:size=1920x1080:rate=30 \
  -c:v libx265 -pix_fmt yuv420p10le -preset fast \
  -y /data/sample_10bit_hevc.mkv

if [ -f "sample_10bit_hevc.mkv" ]; then
    echo "✓ Created sample_10bit_hevc.mkv"
    ls -lh sample_10bit_hevc.mkv
    
    # Verify with ffprobe
    echo "  Verifying..."
    docker run --rm -v "$(pwd):/data" \
      --entrypoint ffprobe \
      lscr.io/linuxserver/ffmpeg:version-8.0-cli \
      -v error -select_streams v:0 \
      -show_entries stream=codec_name,pix_fmt,bit_depth \
      -of default=noprint_wrappers=1 /data/sample_10bit_hevc.mkv
else
    echo "✗ Failed to create 10-bit sample"
fi
echo ""

echo "==================================="
echo "Sample Creation Complete"
echo "==================================="
echo ""
echo "Files created:"
ls -lh sample_*.{mp4,mkv} 2>/dev/null || echo "No samples found"
echo ""
echo "Next step: Run integration tests"
echo "  ./integration_test_qsv.sh sample_8bit_h264.mp4 sample_10bit_hevc.mkv"
