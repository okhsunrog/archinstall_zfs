#!/bin/bash
# Generate synthetic .pkg.tar.zst files for the download bench.
#
# Sizes follow the distribution observed in a typical Arch install (sampled
# from a 6121-package real cache):
#
#   ~32% < 100KB    (small headers, themes, icons)
#   ~35% 100KB-1MB  (typical libraries and small apps)
#   ~21% 1MB-10MB   (medium apps and libs)
#   ~12% 10MB-50MB  (firmware, large apps; capped at 50MB for bench speed)
#
# Total ~50 packages, ~250MB. Takes a few seconds to generate.
#
# Output:
#   packages/<filename>.pkg.tar.zst    synthetic random bytes
#   packages/manifest.json             {filename, size, sha256} list

set -euo pipefail

cd "$(dirname "$0")"
mkdir -p packages
cd packages

# Wipe any previous run so manifest matches what's on disk
rm -f -- *.pkg.tar.zst manifest.json

# Helper: write `count` files of `bytes` bytes with a given name prefix.
gen() {
    local name="$1" count="$2" bytes="$3"
    for i in $(seq -f "%03g" 1 "$count"); do
        local fname="${name}-${i}.pkg.tar.zst"
        head -c "$bytes" /dev/urandom > "$fname"
    done
}

echo "Generating bench packages..."

# Small (16 files, ~30-90KB each, ~960KB total)
gen "small-a" 8  30720    # 30KB
gen "small-b" 8  90112    # 88KB

# Medium (18 files, ~150KB-700KB each, ~7MB total)
gen "medium-a" 6 153600   # 150KB
gen "medium-b" 6 358400   # 350KB
gen "medium-c" 6 716800   # 700KB

# Large (11 files, 1-8MB each, ~38MB total)
gen "large-a" 4 1048576   # 1MB
gen "large-b" 4 4194304   # 4MB
gen "large-c" 3 8388608   # 8MB

# Huge (5 files, 15-50MB each, ~155MB total)
gen "huge-a" 2 15728640   # 15MB
gen "huge-b" 2 31457280   # 30MB
gen "huge-c" 1 52428800   # 50MB

# Build manifest.json with sha256 + size for every file
# (the bench needs sha256 to exercise the verification path).
echo "Hashing and building manifest..."
{
    echo "["
    first=1
    for f in *.pkg.tar.zst; do
        size=$(stat -c %s "$f")
        sha=$(sha256sum "$f" | awk '{print $1}')
        if [ $first -eq 1 ]; then
            first=0
        else
            echo ","
        fi
        printf '  {"filename": "%s", "size": %s, "sha256": "%s"}' "$f" "$size" "$sha"
    done
    echo
    echo "]"
} > manifest.json

count=$(ls -1 *.pkg.tar.zst | wc -l)
total=$(du -sh . | awk '{print $1}')
echo "Generated $count files, total $total"
echo "Manifest: $(pwd)/manifest.json"
