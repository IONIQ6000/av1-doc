#!/bin/bash
# Quick verification script for QSV implementation
# This checks that the code changes are correctly implemented

echo "==================================="
echo "QSV Implementation Verification"
echo "==================================="
echo ""

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0

check_pass() {
    echo -e "${GREEN}✓ PASS${NC}: $1"
    ((PASS++))
}

check_fail() {
    echo -e "${RED}✗ FAIL${NC}: $1"
    ((FAIL++))
}

check_warn() {
    echo -e "${YELLOW}⚠ WARN${NC}: $1"
}

# Check 1: Verify function rename
echo "Checking function rename (run_av1_vaapi_job → run_av1_qsv_job)..."
if grep -q "pub async fn run_av1_qsv_job" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "Function renamed to run_av1_qsv_job"
else
    check_fail "Function not renamed or not found"
fi

if grep -q "run_av1_vaapi_job" crates/daemon/src/ffmpeg_docker.rs; then
    check_fail "Old function name still present"
else
    check_pass "Old function name removed"
fi
echo ""

# Check 2: Verify QSV hardware initialization
echo "Checking QSV hardware initialization..."
if grep -q "qsv=hw:/dev/dri/renderD128" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "QSV hardware initialization present"
else
    check_fail "QSV hardware initialization not found"
fi

if grep -q "vaapi=va:/dev/dri/renderD128" crates/daemon/src/ffmpeg_docker.rs; then
    check_fail "Old VAAPI initialization still present"
else
    check_pass "VAAPI initialization removed"
fi
echo ""

# Check 3: Verify codec change
echo "Checking codec selection..."
if grep -q "av1_qsv" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "av1_qsv codec present"
else
    check_fail "av1_qsv codec not found"
fi

if grep -q "av1_vaapi" crates/daemon/src/ffmpeg_docker.rs; then
    check_fail "Old av1_vaapi codec still present"
else
    check_pass "av1_vaapi codec removed"
fi
echo ""

# Check 4: Verify quality parameter
echo "Checking quality parameter..."
if grep -q "global_quality" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "global_quality parameter present"
else
    check_fail "global_quality parameter not found"
fi

if grep -q '"-qp"' crates/daemon/src/ffmpeg_docker.rs; then
    check_fail "Old -qp parameter still present"
else
    check_pass "-qp parameter removed"
fi
echo ""

# Check 5: Verify environment variable
echo "Checking LIBVA_DRIVER_NAME environment variable..."
if grep -q "LIBVA_DRIVER_NAME" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "LIBVA_DRIVER_NAME environment variable present"
else
    check_fail "LIBVA_DRIVER_NAME not found"
fi

if grep -q "iHD" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "iHD driver specified"
else
    check_fail "iHD driver not specified"
fi
echo ""

# Check 6: Verify Docker image update
echo "Checking Docker image configuration..."
if grep -q "lscr.io/linuxserver/ffmpeg:version-8.0-cli" crates/daemon/src/config.rs; then
    check_pass "New Docker image in config"
else
    check_warn "New Docker image not found in config (may be in different location)"
fi
echo ""

# Check 7: Verify filter chain
echo "Checking filter chain updates..."
if grep -q "hwupload" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "hwupload filter present"
else
    check_fail "hwupload filter not found"
fi

if grep -q "extra_hw_frames" crates/daemon/src/ffmpeg_docker.rs; then
    check_fail "extra_hw_frames parameter still present (should be removed for QSV)"
else
    check_pass "extra_hw_frames parameter removed"
fi
echo ""

# Check 8: Verify profile handling
echo "Checking AV1 profile handling..."
if grep -q "profile:v" crates/daemon/src/ffmpeg_docker.rs; then
    check_pass "Profile parameter present"
else
    check_warn "Profile parameter not found (may be handled differently)"
fi
echo ""

# Check 9: Verify function call sites updated
echo "Checking function call sites..."
if grep -q "run_av1_qsv_job" crates/daemon/src/job.rs 2>/dev/null; then
    check_pass "Function called in job.rs"
elif grep -q "run_av1_qsv_job" crates/daemon/src/lib.rs 2>/dev/null; then
    check_pass "Function called in lib.rs"
else
    check_warn "Function call site not found (may be in different location)"
fi

if grep -q "run_av1_vaapi_job" crates/daemon/src/*.rs 2>/dev/null; then
    check_fail "Old function name still called somewhere"
else
    check_pass "Old function name not called"
fi
echo ""

# Check 10: Verify tests exist
echo "Checking for property-based tests..."
if [ -d "crates/daemon/src" ]; then
    if grep -r "Property.*QSV" crates/daemon/src/ 2>/dev/null | grep -q "Feature: av1-qsv-migration"; then
        check_pass "Property-based tests found"
    else
        check_warn "Property-based tests not found or not tagged correctly"
    fi
else
    check_warn "Cannot check for tests"
fi
echo ""

# Summary
echo "==================================="
echo "Verification Summary"
echo "==================================="
echo -e "${GREEN}Passed: $PASS${NC}"
echo -e "${RED}Failed: $FAIL${NC}"
echo ""

if [ $FAIL -eq 0 ]; then
    echo -e "${GREEN}✓ All critical checks passed!${NC}"
    echo ""
    echo "The code changes appear to be correctly implemented."
    echo "You can now proceed with integration testing using:"
    echo "  ./integration_test_qsv.sh"
    exit 0
else
    echo -e "${RED}✗ Some checks failed${NC}"
    echo ""
    echo "Please review the failed checks above and ensure all"
    echo "code changes are properly implemented before testing."
    exit 1
fi
