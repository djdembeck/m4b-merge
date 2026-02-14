#!/bin/bash
# Generate test audio files for m4b-merge integration tests
# Uses FFmpeg to create short synthetic audio files

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DATA_DIR="${SCRIPT_DIR}/test_data"

echo "Generating test audio files in ${TEST_DATA_DIR}..."

# Create test data directories
mkdir -p "${TEST_DATA_DIR}/single_mp3"
mkdir -p "${TEST_DATA_DIR}/single_m4b"
mkdir -p "${TEST_DATA_DIR}/multiple_mp3s/audiobook"
mkdir -p "${TEST_DATA_DIR}/multi_disc/CD1"
mkdir -p "${TEST_DATA_DIR}/multi_disc/CD2"

# Check if FFmpeg is available
if ! command -v ffmpeg &> /dev/null; then
    echo "Error: FFmpeg not found. Please install FFmpeg to generate test data."
    exit 1
fi

# Generate a short MP3 file with sine wave
# Usage: generate_mp3 <output_path> <duration_seconds> <frequency>
generate_mp3() {
    local output="$1"
    local duration="${2:-5}"
    local freq="${3:-1000}"

    ffmpeg -f lavfi -i "sine=frequency=${freq}:duration=${duration}" \
        -acodec libmp3lame -b:a 128k -ar 44100 -ac 2 \
        -y "${output}" 2>/dev/null
    echo "Generated: ${output}"
}

# Generate a short M4A file with sine wave
# Usage: generate_m4a <output_path> <duration_seconds> <frequency>
generate_m4a() {
    local output="$1"
    local duration="${2:-5}"
    local freq="${3:-1000}"

    ffmpeg -f lavfi -i "sine=frequency=${freq}:duration=${duration}" \
        -acodec aac -b:a 128k -ar 44100 -ac 2 \
        -y "${output}" 2>/dev/null
    echo "Generated: ${output}"
}

# Generate M4B file (same format as M4A, just different extension)
# Usage: generate_m4b <output_path> <duration_seconds> <frequency>
generate_m4b() {
    local output="$1"
    local duration="${2:-5}"
    local freq="${3:-1000}"

    ffmpeg -f lavfi -i "sine=frequency=${freq}:duration=${duration}" \
        -acodec aac -b:a 128k -ar 44100 -ac 2 \
        -y "${output}" 2>/dev/null
    echo "Generated: ${output}"
}

echo ""
echo "=== Generating single MP3 test file ==="
generate_mp3 "${TEST_DATA_DIR}/single_mp3/chapter1.mp3" 10 1000

echo ""
echo "=== Generating multiple MP3 test files ==="
generate_mp3 "${TEST_DATA_DIR}/multiple_mp3s/audiobook/chapter01.mp3" 5 1000
generate_mp3 "${TEST_DATA_DIR}/multiple_mp3s/audiobook/chapter02.mp3" 5 1200
generate_mp3 "${TEST_DATA_DIR}/multiple_mp3s/audiobook/chapter03.mp3" 5 1400
generate_mp3 "${TEST_DATA_DIR}/multiple_mp3s/audiobook/chapter04.mp3" 5 1600
generate_mp3 "${TEST_DATA_DIR}/multiple_mp3s/audiobook/chapter05.mp3" 5 1800

echo ""
echo "=== Generating multi-disc M4A test files ==="
generate_m4a "${TEST_DATA_DIR}/multi_disc/CD1/track01.m4a" 5 1000
generate_m4a "${TEST_DATA_DIR}/multi_disc/CD1/track02.m4a" 5 1100
generate_m4a "${TEST_DATA_DIR}/multi_disc/CD2/track01.m4a" 5 1200
generate_m4a "${TEST_DATA_DIR}/multi_disc/CD2/track02.m4a" 5 1300

echo ""
echo "=== Generating M4B test file ==="
generate_m4b "${TEST_DATA_DIR}/single_m4b/audiobook.m4b" 15 1000

echo ""
echo "=== Test data generation complete ==="
echo "Test data location: ${TEST_DATA_DIR}"
echo ""
echo "Generated files:"
find "${TEST_DATA_DIR}" -type f \( -name "*.mp3" -o -name "*.m4a" -o -name "*.m4b" \) | sort
