# Manual ZFS Package Build Test Commands

Run these commands one by one to test the ZFS package build process manually:

## 1. Check Current System State
```bash
# Check current kernel version
pacman -Si linux-lts | grep Version

# Check what ZFS packages are available
curl -s https://archzfs.com/archzfs/x86_64/ | grep -o 'zfs-linux-lts-[^"]*\.pkg\.tar\.zst' | head -5

# Check current directory
pwd
ls -la
```

## 2. Set Up Build Environment
```bash
# Create build directories
mkdir -p local_repo/zfs-utils-build
mkdir -p local_repo/zfs-linux-lts-build
mkdir -p artifacts

# Clone zfs-utils AUR package
cd local_repo
git clone https://aur.archlinux.org/zfs-utils.git zfs-utils-build
cd zfs-utils-build

# Check what version we're building
grep -E "pkgver|pkgrel" PKGBUILD
```

## 3. Install Build Dependencies (if needed)
```bash
# Install basic build tools
sudo pacman -S --noconfirm --needed base-devel git

# Install ZFS-specific build dependencies
sudo pacman -S --noconfirm --needed \
    archlinux-keyring gnupg bc flex bison openssl zlib perl elfutils \
    git curl python util-linux autoconf automake libtool which grep sed awk

# Try to install kernel headers (this might fail due to version mismatch)
sudo pacman -S --noconfirm --needed linux-lts-headers || echo "Headers install failed - expected"
```

## 4. Test zfs-utils Build
```bash
# Go to zfs-utils build directory
cd ~/code/archinstall_zfs/local_repo/zfs-utils-build

# Check PKGBUILD content
cat PKGBUILD | head -20

# Try building with dependency checking (this will likely fail)
makepkg -s --noconfirm --log --nocheck --skippgpcheck

# If that fails, try without dependency checking
makepkg --noconfirm --log --nocheck --skippgpcheck --nodeps

# Check what happened
echo "Exit code: $?"
ls -la *.pkg.tar.* 2>/dev/null || echo "No packages built"
ls -la *.log 2>/dev/null || echo "No log files"
```

## 5. If zfs-utils Build Fails, Check Logs
```bash
# Look for makepkg log
find . -name "*.log" -exec echo "=== {} ===" \; -exec cat {} \;

# Check for config.log (detailed build errors)
find . -name "config.log" -exec echo "=== {} ===" \; -exec tail -100 {} \;

# Check PKGBUILD for dependencies
grep -A 10 -B 5 "depends\|makedepends" PKGBUILD
```

## 6. Test zfs-linux-lts Build (if zfs-utils succeeds)
```bash
# Go to zfs-linux-lts build directory
cd ~/code/archinstall_zfs/local_repo
git clone https://aur.archlinux.org/zfs-linux-lts.git zfs-linux-lts-build
cd zfs-linux-lts-build

# Check what kernel version it expects
grep -E "_kernelver|_zfsver" PKGBUILD

# Check dependencies
grep -A 10 -B 5 "depends\|makedepends" PKGBUILD

# Try building
makepkg --noconfirm --log --nocheck --skippgpcheck --nodeps

# Check results
echo "Exit code: $?"
ls -la *.pkg.tar.* 2>/dev/null || echo "No packages built"
find . -name "*.log" -exec echo "=== {} ===" \; -exec cat {} \;
```

## 7. Alternative: Check What the Python Script is Actually Doing
```bash
# Go back to project root
cd ~/code/archinstall_zfs

# Run with maximum debugging
PYTHONUNBUFFERED=1 python3 -u gen_iso/build_zfs_package.py 2>&1 | tee build_debug.log

# Or run our debug wrapper
python3 debug_zfs_build.py 2>&1 | tee debug_output.log
```

## 8. Check for Specific Error Patterns
```bash
# Look for dependency errors
grep -i "error.*target not found\|missing dependencies\|could not resolve" build_debug.log

# Look for actual makepkg errors
grep -A 5 -B 5 "ERROR\|FAILED\|error:" build_debug.log

# Check if it's a kernel version mismatch
grep -i "linux-lts.*not found\|headers.*not found" build_debug.log
```

## 9. Quick Fix Test: Force Current Kernel Version
```bash
# Check what kernel version is actually available
pacman -Ss linux-lts | grep "^extra/linux-lts "

# If the PKGBUILD expects a different version, we can try editing it
cd local_repo/zfs-linux-lts-build
cp PKGBUILD PKGBUILD.orig

# Edit the PKGBUILD to use current kernel version (you'll need to do this manually)
# Look for _kernelver= line and update it to match your system
```

## 10. Minimal Test: Just Check What Fails
```bash
# Go to the failing build directory
cd ~/code/archinstall_zfs/local_repo/zfs-utils-build

# Run makepkg with maximum verbosity
makepkg --noconfirm --log --nocheck --skippgpcheck --nodeps -v 2>&1 | tee makepkg_verbose.log

# Check the last few lines for the actual error
tail -50 makepkg_verbose.log
```

---

**Start with commands 1-4, then run command 4 (the makepkg test) and let me know exactly what error message you get. That will tell us the real issue.**