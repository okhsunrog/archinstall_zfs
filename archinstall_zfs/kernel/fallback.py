"""
Fallback strategy and enhanced ZFS installer.

This module provides the FallbackStrategy class that defines intelligent
fallback behavior and the EnhancedZFSInstaller that orchestrates the
installation process with proper error handling.
"""

from __future__ import annotations

from typing import Any

from archinstall import debug, error, info, warn

from ..menu.models import ZFSModuleMode
from .package_manager import InstallationResult, ZFSPackageManager
from .registry import KernelRegistry
from .variants import KernelVariant


class FallbackStrategy:
    """Defines fallback behavior for ZFS installation.
    
    This class implements the core principle: NEVER change kernel variant
    during fallback. Only change ZFS module mode from precompiled to DKMS.
    """
    
    @staticmethod
    def get_fallback_chain(
        variant: KernelVariant, 
        requested_mode: ZFSModuleMode
    ) -> list[tuple[KernelVariant, ZFSModuleMode]]:
        """Get fallback chain for installation attempts.
        
        Key principle: NEVER change kernel variant during fallback.
        Only change ZFS module mode.
        
        Args:
            variant: The kernel variant to use
            requested_mode: The requested ZFS module mode
            
        Returns:
            List of (variant, mode) tuples to try in order
        """
        chain = []
        
        if requested_mode == ZFSModuleMode.PRECOMPILED:
            if variant.supports_precompiled:
                # Try precompiled first
                chain.append((variant, ZFSModuleMode.PRECOMPILED))
            
            # Always fallback to DKMS with SAME kernel
            chain.append((variant, ZFSModuleMode.DKMS))
        else:
            # Direct DKMS request
            chain.append((variant, ZFSModuleMode.DKMS))
        
        return chain
    
    @staticmethod
    def should_attempt_precompiled(variant: KernelVariant, requested_mode: ZFSModuleMode) -> bool:
        """Determine if precompiled installation should be attempted.
        
        Args:
            variant: The kernel variant
            requested_mode: The requested ZFS module mode
            
        Returns:
            True if precompiled should be attempted, False otherwise
        """
        return (
            requested_mode == ZFSModuleMode.PRECOMPILED and 
            variant.supports_precompiled and 
            variant.zfs_precompiled_package is not None
        )
    
    @staticmethod
    def get_recommended_mode(variant: KernelVariant) -> ZFSModuleMode:
        """Get the recommended ZFS module mode for a kernel variant.
        
        Args:
            variant: The kernel variant
            
        Returns:
            The recommended ZFS module mode
        """
        if variant.supports_precompiled:
            return ZFSModuleMode.PRECOMPILED
        else:
            return ZFSModuleMode.DKMS


class EnhancedZFSInstaller:
    """Enhanced installer with proper fallback logic.
    
    This class orchestrates the ZFS installation process, handling fallback
    scenarios gracefully while maintaining kernel consistency.
    """
    
    def __init__(self, kernel_registry: KernelRegistry) -> None:
        """Initialize the enhanced installer.
        
        Args:
            kernel_registry: The kernel registry to use
        """
        self.kernel_registry = kernel_registry
        self.package_manager = ZFSPackageManager(kernel_registry)
        self.fallback_strategy = FallbackStrategy()
    
    def install_with_fallback(
        self, 
        kernel_name: str, 
        preferred_mode: ZFSModuleMode,
        installation: Any = None
    ) -> InstallationResult:
        """Install ZFS with comprehensive fallback logic.
        
        This method attempts to install ZFS packages using the preferred mode,
        with intelligent fallback that maintains kernel consistency.
        
        Args:
            kernel_name: Name of the kernel variant
            preferred_mode: Preferred ZFS module mode
            installation: Installation context (None for host system)
            
        Returns:
            InstallationResult with details of the installation
        """
        variant = self.kernel_registry.get_variant(kernel_name)
        if not variant:
            error(f"Unsupported kernel: {kernel_name}")
            result = InstallationResult(
                kernel_variant=KernelVariant(
                    name=kernel_name,
                    display_name=kernel_name,
                    kernel_package=kernel_name,
                    headers_package=f"{kernel_name}-headers",
                    zfs_precompiled_package=None,
                    supports_precompiled=False
                ),
                requested_mode=preferred_mode
            )
            result.add_error(f"Unsupported kernel: {kernel_name}")
            return result
        
        # Get the fallback chain
        fallback_chain = self.fallback_strategy.get_fallback_chain(variant, preferred_mode)
        
        info(f"Installing ZFS for {variant.display_name} with preferred mode: {preferred_mode.value}")
        debug(f"Fallback chain: {[(v.name, m.value) for v, m in fallback_chain]}")
        
        last_result = None
        for attempt_num, (attempt_variant, attempt_mode) in enumerate(fallback_chain, 1):
            info(f"Attempt {attempt_num}: {attempt_variant.name} with {attempt_mode.value}")
            
            result = self.package_manager.install_zfs_packages(
                attempt_variant.name,
                attempt_mode,
                installation
            )
            
            if result.success:
                if attempt_mode != preferred_mode:
                    result.fallback_occurred = True
                    info(f"Fallback successful: {preferred_mode.value} → {attempt_mode.value}")
                else:
                    info(f"Installation successful with preferred mode: {attempt_mode.value}")
                
                return result
            
            last_result = result
            warn(f"Installation attempt {attempt_num} failed: {result.get_summary()}")
        
        # All attempts failed
        error("All ZFS installation attempts failed")
        return last_result or InstallationResult(
            kernel_variant=variant,
            requested_mode=preferred_mode,
            success=False
        )
    
    def get_installation_summary(self, kernel_name: str, preferred_mode: ZFSModuleMode) -> str:
        """Get a summary of what would be installed.
        
        Args:
            kernel_name: Name of the kernel variant
            preferred_mode: Preferred ZFS module mode
            
        Returns:
            Human-readable summary of the installation plan
        """
        variant = self.kernel_registry.get_variant(kernel_name)
        if not variant:
            return f"ERROR: Unsupported kernel: {kernel_name}"
        
        fallback_chain = self.fallback_strategy.get_fallback_chain(variant, preferred_mode)
        
        summary_lines = [
            f"Installation plan for {variant.display_name}:",
            f"Requested mode: {preferred_mode.value}"
        ]
        
        for attempt_num, (attempt_variant, attempt_mode) in enumerate(fallback_chain, 1):
            if attempt_mode == ZFSModuleMode.PRECOMPILED:
                packages = attempt_variant.get_precompiled_packages()
            else:
                packages = attempt_variant.get_dkms_packages()
            
            mode_text = "primary" if attempt_num == 1 else "fallback"
            summary_lines.append(
                f"  {mode_text.capitalize()}: {attempt_mode.value} - {', '.join(packages)}"
            )
        
        return "\n".join(summary_lines)
    
    def validate_installation_plan(self, kernel_name: str, preferred_mode: ZFSModuleMode) -> list[str]:
        """Validate that an installation plan is feasible.
        
        Args:
            kernel_name: Name of the kernel variant
            preferred_mode: Preferred ZFS module mode
            
        Returns:
            List of validation errors (empty if valid)
        """
        errors = []
        
        variant = self.kernel_registry.get_variant(kernel_name)
        if not variant:
            errors.append(f"Unsupported kernel: {kernel_name}")
            return errors
        
        # Check if preferred mode is supported
        if preferred_mode == ZFSModuleMode.PRECOMPILED and not variant.supports_precompiled:
            errors.append(
                f"Kernel {variant.name} does not support precompiled ZFS modules. "
                "DKMS will be used instead."
            )
        
        # Validate that DKMS packages exist (fallback should always work)
        try:
            dkms_packages = variant.get_dkms_packages()
            if not self.package_manager.verify_packages_available(dkms_packages):
                errors.append(
                    f"Required DKMS packages not available: {', '.join(dkms_packages)}"
                )
        except Exception as e:
            errors.append(f"Failed to validate DKMS packages: {e}")
        
        return errors
    
    def detect_running_kernel_variant(self) -> str:
        """Detect the kernel variant of the currently running kernel.
        
        Returns:
            The name of the detected kernel variant
        """
        try:
            from archinstall.lib.general import SysCommand
            kernel_version = SysCommand("uname -r").decode().strip()
            
            # Use simple heuristics to detect kernel variant
            if "lts" in kernel_version.lower():
                return "linux-lts"
            elif "zen" in kernel_version.lower():
                return "linux-zen"
            elif "hardened" in kernel_version.lower():
                return "linux-hardened"
            elif "rt" in kernel_version.lower():
                if "lts" in kernel_version.lower():
                    return "linux-rt-lts"
                else:
                    return "linux-rt"
            else:
                return "linux"
                
        except Exception as e:
            warn(f"Failed to detect running kernel variant: {e}")
            return "linux-lts"  # Safe default
    
    def get_recommended_configuration(self) -> tuple[str, ZFSModuleMode]:
        """Get recommended kernel and ZFS mode configuration.
        
        Returns:
            Tuple of (kernel_name, zfs_mode) representing the recommended configuration
        """
        # Try to detect running kernel first
        running_kernel = self.detect_running_kernel_variant()
        variant = self.kernel_registry.get_variant(running_kernel)
        
        if variant:
            recommended_mode = self.fallback_strategy.get_recommended_mode(variant)
            return running_kernel, recommended_mode
        
        # Fall back to default variant
        default_variant = self.kernel_registry.get_default_variant()
        if default_variant:
            recommended_mode = self.fallback_strategy.get_recommended_mode(default_variant)
            return default_variant.name, recommended_mode
        
        # Ultimate fallback
        return "linux-lts", ZFSModuleMode.PRECOMPILED