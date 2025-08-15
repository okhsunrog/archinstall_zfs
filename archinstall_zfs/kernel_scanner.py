"""
Kernel compatibility scanner for ZFS installations.

This module scans for available kernels and their ZFS compatibility
before the menu is displayed, caching results for efficient access.
"""

from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

from archinstall import debug, info, warn

from archinstall_zfs.kernel import AVAILABLE_KERNELS, KernelInfo
from archinstall_zfs.shared import ZFSModuleMode


@dataclass
class CompatibilityResult:
    """Result of compatibility check for a kernel."""
    kernel_name: str
    kernel_info: KernelInfo
    dkms_compatible: bool
    dkms_warnings: List[str]
    precompiled_compatible: bool
    precompiled_warnings: List[str]


class KernelCompatibilityScanner:
    """
    Scans and caches kernel compatibility information.
    
    This class performs upfront scanning of all available kernels
    for both DKMS and precompiled ZFS compatibility, caching the
    results for efficient menu generation.
    """
    
    def __init__(self):
        self._results: Dict[str, CompatibilityResult] = {}
        self._scanned = False
        self._filtering_enabled = True
    
    def _refresh_pacman_database(self) -> None:
        """Refresh pacman sync database to ensure current package information."""
        info("📦 Refreshing pacman database...")
        try:
            from archinstall.lib.general import SysCommand
            # Refresh sync database without downloading packages
            SysCommand("pacman -Sy")
            info("✅ Pacman database refreshed successfully")
        except Exception as e:
            warn(f"Failed to refresh pacman database: {e}")
            warn("Package version detection may be unreliable")
    
    def _test_package_detection(self) -> None:
        """Test package detection for debugging purposes."""
        debug("🧪 Testing package detection...")
        try:
            from archinstall_zfs.validation import get_package_version
            
            test_packages = ["linux-lts", "linux", "zfs-dkms"]
            for pkg in test_packages:
                version = get_package_version(pkg)
                debug(f"  {pkg}: {version if version else 'NOT FOUND'}")
                
        except Exception as e:
            debug(f"Package detection test failed: {e}")
    
    def scan_compatibility(self, enable_filtering: bool = True) -> None:
        """
        Scan all available kernels for ZFS compatibility.
        
        Args:
            enable_filtering: Whether to enable compatibility filtering
        """
        self._filtering_enabled = enable_filtering
        self._results.clear()
        
        info("🔍 Scanning kernel compatibility for ZFS...")
        info(f"Filtering enabled: {enable_filtering}")
        
        if not enable_filtering:
            info("⚠️  Compatibility filtering disabled - all options will be shown")
        
        # Refresh pacman sync database to ensure we have current package info
        self._refresh_pacman_database()
        
        # Test package detection for debugging
        self._test_package_detection()
        
        # Import validation functions here to avoid circular imports
        try:
            from archinstall_zfs.validation import validate_kernel_zfs_compatibility, validate_precompiled_zfs_compatibility
        except ImportError:
            warn("Failed to import validation functions - using fallback")
            self._create_fallback_results()
            return
        
        for kernel_name, kernel_info in AVAILABLE_KERNELS.items():
            info(f"Checking compatibility for {kernel_name}...")
            
            # Check DKMS compatibility
            dkms_compatible = True
            dkms_warnings = []
            
            if enable_filtering:
                try:
                    dkms_compatible, dkms_warnings = validate_kernel_zfs_compatibility(kernel_name, "dkms")
                    debug(f"DKMS {kernel_name}: compatible={dkms_compatible}, warnings={len(dkms_warnings)}")
                    
                    # If validation failed due to missing package info, fall back to compatible
                    if not dkms_compatible and any("Could not determine" in w for w in dkms_warnings):
                        warn(f"Package detection failed for {kernel_name} - assuming DKMS compatible")
                        dkms_compatible = True
                        dkms_warnings = ["Package version detection failed - assuming compatible"]
                        
                except Exception as e:
                    warn(f"DKMS validation failed for {kernel_name}: {e}")
                    dkms_compatible = True  # Fail open for robustness
                    dkms_warnings = [f"Validation error: {e} - assuming compatible"]
            
            # Check precompiled compatibility
            precompiled_compatible = True
            precompiled_warnings = []
            
            if enable_filtering and kernel_info.precompiled_package:
                try:
                    precompiled_compatible, precompiled_warnings = validate_precompiled_zfs_compatibility(kernel_name)
                    debug(f"Precompiled {kernel_name}: compatible={precompiled_compatible}, warnings={len(precompiled_warnings)}")
                    
                    # If validation failed due to missing package info, fall back to compatible  
                    if not precompiled_compatible and any("Could not determine" in w for w in precompiled_warnings):
                        warn(f"Package detection failed for {kernel_name} - assuming precompiled compatible")
                        precompiled_compatible = True
                        precompiled_warnings = ["Package version detection failed - assuming compatible"]
                        
                except Exception as e:
                    warn(f"Precompiled validation failed for {kernel_name}: {e}")
                    precompiled_compatible = True  # Fail open for robustness
                    precompiled_warnings = [f"Validation error: {e} - assuming compatible"]
            elif not kernel_info.precompiled_package:
                precompiled_compatible = False
                precompiled_warnings = ["No precompiled package available"]
            
            # Store result
            result = CompatibilityResult(
                kernel_name=kernel_name,
                kernel_info=kernel_info,
                dkms_compatible=dkms_compatible,
                dkms_warnings=dkms_warnings,
                precompiled_compatible=precompiled_compatible,
                precompiled_warnings=precompiled_warnings,
            )
            
            self._results[kernel_name] = result
            
            # Log summary for this kernel
            modes = []
            if precompiled_compatible:
                modes.append("precompiled")
            if dkms_compatible:
                modes.append("DKMS")
            
            if modes:
                info(f"✅ {kernel_name}: {', '.join(modes)} available")
            else:
                info(f"❌ {kernel_name}: no compatible ZFS options")
                
            # Log warnings
            for warning in dkms_warnings + precompiled_warnings:
                debug(f"  Warning: {warning}")
        
        self._scanned = True
        self._log_summary()
    
    def _create_fallback_results(self) -> None:
        """Create fallback results when validation functions are unavailable."""
        warn("Using fallback compatibility results - all kernels marked as compatible")
        
        for kernel_name, kernel_info in AVAILABLE_KERNELS.items():
            result = CompatibilityResult(
                kernel_name=kernel_name,
                kernel_info=kernel_info,
                dkms_compatible=True,
                dkms_warnings=["Validation unavailable - assuming compatible"],
                precompiled_compatible=bool(kernel_info.precompiled_package),
                precompiled_warnings=[] if kernel_info.precompiled_package else ["No precompiled package available"],
            )
            self._results[kernel_name] = result
        
        self._scanned = True
    
    def _log_summary(self) -> None:
        """Log a summary of the compatibility scan results."""
        if not self._scanned:
            return
            
        total_kernels = len(self._results)
        dkms_compatible = sum(1 for r in self._results.values() if r.dkms_compatible)
        precompiled_compatible = sum(1 for r in self._results.values() if r.precompiled_compatible)
        
        info(f"📊 Compatibility scan complete:")
        info(f"  Total kernels: {total_kernels}")
        info(f"  DKMS compatible: {dkms_compatible}")
        info(f"  Precompiled compatible: {precompiled_compatible}")
        
        if self._filtering_enabled:
            total_options = sum(
                (1 if r.dkms_compatible else 0) + (1 if r.precompiled_compatible else 0)
                for r in self._results.values()
            )
            info(f"  Total menu options: {total_options}")
            
            if total_options == 0:
                warn("⚠️  NO COMPATIBLE OPTIONS FOUND!")
                warn("This will result in 'No compatible kernel options available' error")
                warn("Consider disabling filtering with: export ARCHINSTALL_ZFS_FILTER_KERNELS=false")
        else:
            total_options = total_kernels * 2  # Each kernel has DKMS + precompiled (if available)
            info(f"  Total menu options (unfiltered): {total_options}")
    
    def get_menu_options(self) -> Tuple[List[Tuple[str, str, ZFSModuleMode]], List[str]]:
        """
        Generate menu options from cached compatibility results.
        
        Returns:
            Tuple of (available_options, filtered_kernels)
        """
        if not self._scanned:
            warn("Compatibility not scanned yet - performing scan now")
            self.scan_compatibility(self._should_filter_from_env())
        
        info(f"🎯 Generating menu options (filtering={'enabled' if self._filtering_enabled else 'disabled'})...")
        
        options = []
        filtered_kernels = []
        
        for kernel_name, result in self._results.items():
            debug(f"Processing {kernel_name}: DKMS={result.dkms_compatible}, precompiled={result.precompiled_compatible}")
            
            # Add precompiled option if compatible (or filtering disabled)
            if result.precompiled_compatible or not self._filtering_enabled:
                if result.kernel_info.precompiled_package:
                    display = f"{result.kernel_info.display_name} + precompiled ZFS"
                    if kernel_name == "linux-lts":
                        display += " (recommended)"
                    options.append((display, kernel_name, ZFSModuleMode.PRECOMPILED))
                    debug(f"  ✅ Added precompiled option: {display}")
            elif result.kernel_info.precompiled_package:
                debug(f"  ❌ Skipped precompiled option for {kernel_name} (incompatible)")
            
            # Add DKMS option if compatible (or filtering disabled)
            if result.dkms_compatible or not self._filtering_enabled:
                display = f"{result.kernel_info.display_name} + ZFS DKMS"
                options.append((display, kernel_name, ZFSModuleMode.DKMS))
                debug(f"  ✅ Added DKMS option: {display}")
            else:
                debug(f"  ❌ Skipped DKMS option for {kernel_name} (incompatible)")
            
            # Track filtered kernels (only DKMS incompatible ones for the notice)
            if self._filtering_enabled and not result.dkms_compatible:
                filtered_kernels.append(result.kernel_info.display_name)
        
        info(f"📋 Generated {len(options)} menu options, {len(filtered_kernels)} filtered")
        
        if len(options) == 0:
            warn("🚨 NO MENU OPTIONS GENERATED!")
            warn("This will cause 'No compatible kernel options available' error")
            warn("Debug info:")
            for kernel_name, result in self._results.items():
                warn(f"  {kernel_name}: DKMS={result.dkms_compatible}, precompiled={result.precompiled_compatible}")
        
        return options, filtered_kernels
    
    def get_compatibility_result(self, kernel_name: str) -> Optional[CompatibilityResult]:
        """Get compatibility result for a specific kernel."""
        return self._results.get(kernel_name)
    
    def is_compatible(self, kernel_name: str, mode: ZFSModuleMode) -> bool:
        """Check if a specific kernel/mode combination is compatible."""
        result = self.get_compatibility_result(kernel_name)
        if not result:
            return False
        
        if mode == ZFSModuleMode.DKMS:
            return result.dkms_compatible or not self._filtering_enabled
        else:  # PRECOMPILED
            return result.precompiled_compatible or not self._filtering_enabled
    
    def _should_filter_from_env(self) -> bool:
        """Check environment variable for filtering preference."""
        try:
            from archinstall_zfs.validation import should_filter_kernel_options
            return should_filter_kernel_options()
        except ImportError:
            # Fallback to checking environment directly
            import os
            filter_env = os.getenv("ARCHINSTALL_ZFS_FILTER_KERNELS", "").lower()
            return filter_env not in ("0", "false", "no", "off", "disable")


# Global scanner instance
_scanner = KernelCompatibilityScanner()


def get_kernel_scanner() -> KernelCompatibilityScanner:
    """Get the global kernel compatibility scanner instance."""
    return _scanner


def scan_kernel_compatibility(enable_filtering: bool = True) -> None:
    """Convenience function to scan kernel compatibility."""
    _scanner.scan_compatibility(enable_filtering)


def get_menu_options() -> Tuple[List[Tuple[str, str, ZFSModuleMode]], List[str]]:
    """Get menu options from the global scanner."""
    return _scanner.get_menu_options()
