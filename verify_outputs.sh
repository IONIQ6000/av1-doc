#!/bin/bash
# Quick script to verify the output files from integration tests

echo "==================================="
echo "Verifying QSV Encoding Outputs"
echo "==================================="
echo ""

# Check if output files exist
if [ ! -f "output_8bit_qsv.mkv" ]; then
    echo "ERROR: output_8bit_qsv.mkv not found"
    echo "Please run integration tests first"
    exit 1
fi

if [ ! -f "output_10bit_qsv.mkv" ]; then
    echo "ERROR: output_10bit_qsv.mkv not found"
    echo "Please run integration tests first"
    exit 1
fi

echo "Checking 8-bit output..."
docker run --rm -v "$(pwd):/data" \
    --entrypoint ffprobe \
    lscr.io/linuxserver/ffmpeg:version-8.0-cli \
    -v error -select_streams v:0 \
    -show_entries stream=codec_name,pix_fmt,width,height \
    -of default=noprint_wrappers=1 /data/output_8bit_qsv.mkv

echo ""
echo "Checking 10-bit output..."
docker run --rm -v "$(pwd):/data" \
    --entrypoint ffprobe \
    lscr.io/linuxserver/ffmpeg:version-8.0-cli \
    -v error -select_streams v:0 \
    -show_entries stream=codec_name,pix_fmt,width,height \
    -of default=noprint_wrappers=1 /data/output_10bit_qsv.mkv

echo ""
echo "==================================="
echo "File Sizes"
echo "==================================="
ls -lh output_*_qsv.mkv

echo ""
echo "==================================="
echo "Expected Results"
echo "==================================="
echo "8-bit output should have:"
echo "  - codec_name=av1"
echo "  - pix_fmt=yuv420p"
echo ""
echo "10-bit output should have:"
echo "  - codec_name=av1"
echo "  - pix_fmt=yuv420p10le"
