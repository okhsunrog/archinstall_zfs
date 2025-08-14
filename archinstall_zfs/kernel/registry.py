"""
Kernel registry for managing supported kernel variants.

This module provides the KernelRegistry class that serves as the central
repository for all supported kernel variants and their configurations.
"""

from __future__ import annotations

import contextlib
import json
from pathlib import Path
from typing import Any

from archinstall import debug, info, warn
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand

from .variants import KernelVariant


class KernelRegistry:
    """Registry for dynamically managing kernel variants.

    This class serves as the central repository for all supported kernel variants,
    providing methods to register, query, and manage kernel configurations.
    """

    def __init__(self) -> None:
        """Initialize the kernel registry with default variants."""
        self._variants: dict[str, KernelVariant] = {}
        self._load_default_variants()

    def register_variant(self, variant: KernelVariant) -> None:
        """Register a new kernel variant.

        Args:
            variant: The kernel variant to register
        """
        if variant.name in self._variants:
            warn(f"Overriding existing kernel variant: {variant.name}")

        self._variants[variant.name] = variant
        info(f"Registered kernel variant: {variant.name}")

    def get_variant(self, name: str) -> KernelVariant | None:
        """Get kernel variant by name.

        Args:
            name: The name of the kernel variant

        Returns:
            The kernel variant if found, None otherwise
        """
        return self._variants.get(name)

    def get_supported_variants(self) -> list[KernelVariant]:
        """Get all supported kernel variants.

        Returns:
            List of all registered kernel variants, sorted by preference
        """
        variants = list(self._variants.values())
        # Sort with default first, then alphabetically
        return sorted(variants, key=lambda v: (not v.is_default, v.name))

    def get_precompiled_variants(self) -> list[KernelVariant]:
        """Get variants that support precompiled ZFS.

        Returns:
            List of kernel variants that support precompiled ZFS modules
        """
        return [v for v in self._variants.values() if v.supports_precompiled]

    def get_default_variant(self) -> KernelVariant | None:
        """Get the default kernel variant.

        Returns:
            The default kernel variant, or None if no default is set
        """
        for variant in self._variants.values():
            if variant.is_default:
                return variant
        return None

    def register_from_config(self, config_data: dict[str, Any]) -> None:
        """Register kernel variants from configuration data.

        Args:
            config_data: Dictionary containing kernel variant configurations
        """
        for variant_data in config_data.get("kernel_variants", []):
            try:
                variant = KernelVariant.from_dict(variant_data)
                self.register_variant(variant)
            except Exception as e:
                warn(f"Failed to register kernel variant from config: {e}")

    def auto_detect_variants(self) -> None:
        """Auto-detect available kernel variants from system.

        This method attempts to detect available kernel packages and
        automatically register variants for them.
        """
        debug("Auto-detecting available kernel variants")

        # Common kernel packages to check for
        potential_kernels = ["linux", "linux-lts", "linux-zen", "linux-hardened", "linux-rt", "linux-rt-lts"]

        for kernel_name in potential_kernels:
            if kernel_name not in self._variants and self._check_package_exists(kernel_name):
                variant = self._create_variant_from_name(kernel_name)
                if variant:
                    self.register_variant(variant)

    def _load_default_variants(self) -> None:
        """Register default supported kernel variants."""

        # Linux LTS - most stable, recommended default
        self.register_variant(
            KernelVariant(
                name="linux-lts",
                display_name="Linux LTS",
                kernel_package="linux-lts",
                headers_package="linux-lts-headers",
                zfs_precompiled_package="zfs-linux-lts",
                supports_precompiled=True,
                is_default=True,
            )
        )

        # Linux - mainline kernel, now with precompiled support
        self.register_variant(
            KernelVariant(
                name="linux",
                display_name="Linux",
                kernel_package="linux",
                headers_package="linux-headers",
                zfs_precompiled_package="zfs-linux",
                supports_precompiled=True,  # Enable precompiled support
                is_default=False,
            )
        )

        # Linux Zen - performance optimized, now with precompiled support
        self.register_variant(
            KernelVariant(
                name="linux-zen",
                display_name="Linux Zen",
                kernel_package="linux-zen",
                headers_package="linux-zen-headers",
                zfs_precompiled_package="zfs-linux-zen",
                supports_precompiled=True,  # Enable precompiled support
                is_default=False,
            )
        )

    def _create_variant_from_name(self, kernel_name: str) -> KernelVariant | None:
        """Create a kernel variant from package name using conventions.

        Args:
            kernel_name: The kernel package name

        Returns:
            A kernel variant created using naming conventions, or None if invalid
        """
        try:
            # Follow Arch Linux naming conventions
            display_name = kernel_name.replace("-", " ").title()
            headers_package = f"{kernel_name}-headers"

            # Check if precompiled ZFS exists
            zfs_package = f"zfs-{kernel_name}"
            supports_precompiled = self._check_package_exists(zfs_package)

            return KernelVariant(
                name=kernel_name,
                display_name=display_name,
                kernel_package=kernel_name,
                headers_package=headers_package,
                zfs_precompiled_package=zfs_package if supports_precompiled else None,
                supports_precompiled=supports_precompiled,
                is_default=kernel_name == "linux-lts",
            )
        except Exception as e:
            warn(f"Failed to create variant for {kernel_name}: {e}")
            return None

    def _check_package_exists(self, package_name: str) -> bool:
        """Check if a package exists in the repositories.

        Args:
            package_name: The package name to check

        Returns:
            True if the package exists, False otherwise
        """
        try:
            # Use pacman to check if package exists
            SysCommand(f"pacman -Si {package_name}", peek_output=True)
            return True
        except SysCallError:
            return False
        except Exception:
            # If pacman is not available or other error, assume package doesn't exist
            return False

    def load_from_file(self, config_path: Path) -> None:
        """Load kernel variants from a configuration file.

        Args:
            config_path: Path to the configuration file
        """
        if not config_path.exists():
            debug(f"Kernel config file not found: {config_path}")
            return

        try:
            with open(config_path) as f:
                config_data = json.load(f)
            self.register_from_config(config_data)
            info(f"Loaded kernel variants from {config_path}")
        except Exception as e:
            warn(f"Failed to load kernel config from {config_path}: {e}")

    def save_to_file(self, config_path: Path) -> None:
        """Save current kernel variants to a configuration file.

        Args:
            config_path: Path where to save the configuration
        """
        try:
            config_data = {"kernel_variants": [v.to_dict() for v in self._variants.values()]}

            config_path.parent.mkdir(parents=True, exist_ok=True)
            with open(config_path, "w") as f:
                json.dump(config_data, f, indent=2)
            info(f"Saved kernel variants to {config_path}")
        except Exception as e:
            warn(f"Failed to save kernel config to {config_path}: {e}")

    def __str__(self) -> str:
        """String representation for debugging."""
        variants = self.get_supported_variants()
        variant_list = "\n".join(f"  - {v}" for v in variants)
        return f"KernelRegistry with {len(variants)} variants:\n{variant_list}"


# Global registry instance
_global_registry: KernelRegistry | None = None


def get_kernel_registry() -> KernelRegistry:
    """Get the global kernel registry instance.

    Returns:
        The global KernelRegistry instance
    """
    global _global_registry
    if _global_registry is None:
        _global_registry = KernelRegistry()

        # Try to load custom configurations
        config_paths = [Path("/etc/archinstall-zfs/kernel-variants.json"), Path.home() / ".config" / "archinstall-zfs" / "kernel-variants.json"]

        for config_path in config_paths:
            with contextlib.suppress(Exception):
                _global_registry.load_from_file(config_path)

        # Auto-detect additional variants
        with contextlib.suppress(Exception):
            _global_registry.auto_detect_variants()

    return _global_registry
