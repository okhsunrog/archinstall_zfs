"""
Comprehensive tests for the enhanced kernel management system.

This test suite validates the new kernel registry, package management,
and fallback logic to ensure proper functionality and backward compatibility.
"""

from typing import Any
from unittest.mock import Mock, patch

import pytest
from archinstall_zfs.kernel.fallback import EnhancedZFSInstaller, FallbackStrategy
from archinstall_zfs.kernel.package_manager import ZFSPackageManager
from archinstall_zfs.kernel.registry import KernelRegistry
from archinstall_zfs.kernel.variants import KernelVariant
from archinstall_zfs.shared import ZFSModuleMode


class TestKernelVariant:
    """Test the KernelVariant dataclass."""

    def test_valid_kernel_variant(self) -> None:
        """Test creating a valid kernel variant."""
        variant = KernelVariant(
            name="linux-lts",
            display_name="Linux LTS",
            kernel_package="linux-lts",
            headers_package="linux-lts-headers",
            zfs_precompiled_package="zfs-linux-lts",
            supports_precompiled=True,
            is_default=True,
        )

        assert variant.name == "linux-lts"
        assert variant.supports_precompiled is True
        assert variant.is_default is True

    def test_invalid_kernel_variant_empty_name(self) -> None:
        """Test that empty name raises ValueError."""
        with pytest.raises(ValueError, match="Kernel variant name cannot be empty"):
            KernelVariant(
                name="",
                display_name="Linux LTS",
                kernel_package="linux-lts",
                headers_package="linux-lts-headers",
                zfs_precompiled_package="zfs-linux-lts",
                supports_precompiled=True,
            )

    def test_invalid_precompiled_config(self) -> None:
        """Test that claiming precompiled support without package raises ValueError."""
        with pytest.raises(ValueError, match="claims to support precompiled ZFS"):
            KernelVariant(
                name="linux-test",
                display_name="Linux Test",
                kernel_package="linux-test",
                headers_package="linux-test-headers",
                zfs_precompiled_package=None,
                supports_precompiled=True,  # This should fail
            )

    def test_get_dkms_packages(self) -> None:
        """Test getting DKMS packages."""
        variant = KernelVariant(
            name="linux-zen",
            display_name="Linux Zen",
            kernel_package="linux-zen",
            headers_package="linux-zen-headers",
            zfs_precompiled_package="zfs-linux-zen",
            supports_precompiled=True,
        )

        packages = variant.get_dkms_packages()
        expected = ["zfs-utils", "zfs-dkms", "linux-zen-headers"]
        assert packages == expected

    def test_get_precompiled_packages(self) -> None:
        """Test getting precompiled packages."""
        variant = KernelVariant(
            name="linux-lts",
            display_name="Linux LTS",
            kernel_package="linux-lts",
            headers_package="linux-lts-headers",
            zfs_precompiled_package="zfs-linux-lts",
            supports_precompiled=True,
        )

        packages = variant.get_precompiled_packages()
        expected = ["zfs-utils", "zfs-linux-lts"]
        assert packages == expected

    def test_get_precompiled_packages_not_supported(self) -> None:
        """Test that getting precompiled packages fails when not supported."""
        variant = KernelVariant(
            name="linux-custom",
            display_name="Linux Custom",
            kernel_package="linux-custom",
            headers_package="linux-custom-headers",
            zfs_precompiled_package=None,
            supports_precompiled=False,
        )

        with pytest.raises(ValueError, match="does not support precompiled ZFS"):
            variant.get_precompiled_packages()


class TestKernelRegistry:
    """Test the KernelRegistry class."""

    def test_default_variants_loaded(self) -> None:
        """Test that default variants are loaded."""
        registry = KernelRegistry()

        # Check that default variants exist
        lts_variant = registry.get_variant("linux-lts")
        assert lts_variant is not None
        assert lts_variant.is_default is True
        assert lts_variant.supports_precompiled is True

        linux_variant = registry.get_variant("linux")
        assert linux_variant is not None
        assert linux_variant.supports_precompiled is True

        zen_variant = registry.get_variant("linux-zen")
        assert zen_variant is not None
        assert zen_variant.supports_precompiled is True

    def test_register_custom_variant(self) -> None:
        """Test registering a custom kernel variant."""
        registry = KernelRegistry()

        custom_variant = KernelVariant(
            name="linux-custom",
            display_name="Linux Custom",
            kernel_package="linux-custom",
            headers_package="linux-custom-headers",
            zfs_precompiled_package=None,
            supports_precompiled=False,
        )

        registry.register_variant(custom_variant)

        retrieved = registry.get_variant("linux-custom")
        assert retrieved is not None
        assert retrieved.name == "linux-custom"
        assert retrieved.supports_precompiled is False

    def test_get_precompiled_variants(self) -> None:
        """Test getting variants that support precompiled ZFS."""
        registry = KernelRegistry()

        precompiled_variants = registry.get_precompiled_variants()
        variant_names = [v.name for v in precompiled_variants]

        # All default variants should support precompiled
        assert "linux-lts" in variant_names
        assert "linux" in variant_names
        assert "linux-zen" in variant_names

    def test_get_default_variant(self) -> None:
        """Test getting the default variant."""
        registry = KernelRegistry()

        default = registry.get_default_variant()
        assert default is not None
        assert default.name == "linux-lts"
        assert default.is_default is True


class TestFallbackStrategy:
    """Test the FallbackStrategy class."""

    def test_precompiled_fallback_chain(self) -> None:
        """Test fallback chain for precompiled request."""
        variant = KernelVariant(
            name="linux-lts",
            display_name="Linux LTS",
            kernel_package="linux-lts",
            headers_package="linux-lts-headers",
            zfs_precompiled_package="zfs-linux-lts",
            supports_precompiled=True,
        )

        chain = FallbackStrategy.get_fallback_chain(variant, ZFSModuleMode.PRECOMPILED)

        # Should try precompiled first, then DKMS with same kernel
        assert len(chain) == 2
        assert chain[0] == (variant, ZFSModuleMode.PRECOMPILED)
        assert chain[1] == (variant, ZFSModuleMode.DKMS)

    def test_dkms_fallback_chain(self) -> None:
        """Test fallback chain for direct DKMS request."""
        variant = KernelVariant(
            name="linux-zen",
            display_name="Linux Zen",
            kernel_package="linux-zen",
            headers_package="linux-zen-headers",
            zfs_precompiled_package="zfs-linux-zen",
            supports_precompiled=True,
        )

        chain = FallbackStrategy.get_fallback_chain(variant, ZFSModuleMode.DKMS)

        # Should only try DKMS
        assert len(chain) == 1
        assert chain[0] == (variant, ZFSModuleMode.DKMS)

    def test_precompiled_not_supported_fallback(self) -> None:
        """Test fallback when precompiled is not supported."""
        variant = KernelVariant(
            name="linux-custom",
            display_name="Linux Custom",
            kernel_package="linux-custom",
            headers_package="linux-custom-headers",
            zfs_precompiled_package=None,
            supports_precompiled=False,
        )

        chain = FallbackStrategy.get_fallback_chain(variant, ZFSModuleMode.PRECOMPILED)

        # Should skip precompiled and go straight to DKMS
        assert len(chain) == 1
        assert chain[0] == (variant, ZFSModuleMode.DKMS)

    def test_should_attempt_precompiled(self) -> None:
        """Test logic for determining if precompiled should be attempted."""
        variant_with_precompiled = KernelVariant(
            name="linux-lts",
            display_name="Linux LTS",
            kernel_package="linux-lts",
            headers_package="linux-lts-headers",
            zfs_precompiled_package="zfs-linux-lts",
            supports_precompiled=True,
        )

        variant_without_precompiled = KernelVariant(
            name="linux-custom",
            display_name="Linux Custom",
            kernel_package="linux-custom",
            headers_package="linux-custom-headers",
            zfs_precompiled_package=None,
            supports_precompiled=False,
        )

        # Should attempt precompiled
        assert FallbackStrategy.should_attempt_precompiled(variant_with_precompiled, ZFSModuleMode.PRECOMPILED) is True

        # Should not attempt precompiled (not supported)
        assert FallbackStrategy.should_attempt_precompiled(variant_without_precompiled, ZFSModuleMode.PRECOMPILED) is False

        # Should not attempt precompiled (DKMS requested)
        assert FallbackStrategy.should_attempt_precompiled(variant_with_precompiled, ZFSModuleMode.DKMS) is False


class TestZFSPackageManager:
    """Test the ZFSPackageManager class."""

    def setup_method(self) -> None:
        """Set up test fixtures."""
        self.registry = KernelRegistry()
        self.package_manager = ZFSPackageManager(self.registry)

    @patch("archinstall_zfs.kernel.package_manager.SysCommand")
    def test_install_precompiled_success(self, mock_syscmd: Any) -> None:  # noqa: ARG002
        """Test successful precompiled installation."""
        # Mock successful installation
        mock_installation = Mock()
        mock_installation.add_additional_packages = Mock()

        result = self.package_manager.install_zfs_packages("linux-lts", ZFSModuleMode.PRECOMPILED, mock_installation)

        assert result.success is True
        assert result.actual_mode == ZFSModuleMode.PRECOMPILED
        assert result.fallback_occurred is False
        assert "zfs-utils" in result.installed_packages

    @patch("archinstall_zfs.kernel.package_manager.SysCommand")
    def test_install_with_fallback(self, mock_syscmd: Any) -> None:  # noqa: ARG002
        """Test installation with fallback from precompiled to DKMS."""
        # Mock failed precompiled, successful DKMS
        mock_installation = Mock()
        mock_installation.add_additional_packages = Mock()
        mock_installation.add_additional_packages.side_effect = [Exception("Precompiled failed"), None]

        with (
            patch.object(self.package_manager, "_try_precompiled_install", return_value=False),
            patch.object(self.package_manager, "_install_dkms", return_value=True),
        ):
            result = self.package_manager.install_zfs_packages("linux-lts", ZFSModuleMode.PRECOMPILED, mock_installation)

        assert result.success is True
        assert result.actual_mode == ZFSModuleMode.DKMS
        assert result.fallback_occurred is True

    def test_install_unsupported_kernel(self) -> None:
        """Test installation with unsupported kernel."""
        result = self.package_manager.install_zfs_packages("linux-nonexistent", ZFSModuleMode.PRECOMPILED, None)

        assert result.success is False
        assert "Unsupported kernel" in result.errors[0]


class TestEnhancedZFSInstaller:
    """Test the EnhancedZFSInstaller class."""

    def setup_method(self) -> None:
        """Set up test fixtures."""
        self.registry = KernelRegistry()
        self.installer = EnhancedZFSInstaller(self.registry)

    def test_detect_running_kernel_variant(self) -> None:
        """Test kernel variant detection."""
        with patch("archinstall_zfs.kernel.fallback.SysCommand") as mock_syscmd:
            # Test LTS detection
            mock_syscmd.return_value.decode.return_value.strip.return_value = "5.15.0-lts"
            assert self.installer.detect_running_kernel_variant() == "linux-lts"

            # Test Zen detection
            mock_syscmd.return_value.decode.return_value.strip.return_value = "5.19.0-zen1"
            assert self.installer.detect_running_kernel_variant() == "linux-zen"

            # Test regular kernel detection
            mock_syscmd.return_value.decode.return_value.strip.return_value = "5.19.0-arch1"
            assert self.installer.detect_running_kernel_variant() == "linux"

    def test_get_recommended_configuration(self) -> None:
        """Test getting recommended configuration."""
        with patch.object(self.installer, "detect_running_kernel_variant", return_value="linux-lts"):
            kernel, mode = self.installer.get_recommended_configuration()
            assert kernel == "linux-lts"
            assert mode == ZFSModuleMode.PRECOMPILED

    def test_validate_installation_plan(self) -> None:
        """Test installation plan validation."""
        # Valid plan should have no errors
        errors = self.installer.validate_installation_plan("linux-lts", ZFSModuleMode.PRECOMPILED)
        assert len(errors) == 0

        # Invalid kernel should have errors
        errors = self.installer.validate_installation_plan("linux-nonexistent", ZFSModuleMode.PRECOMPILED)
        assert len(errors) > 0
        assert "Unsupported kernel" in errors[0]

    def test_get_installation_summary(self) -> None:
        """Test getting installation summary."""
        summary = self.installer.get_installation_summary("linux-lts", ZFSModuleMode.PRECOMPILED)

        assert "Linux LTS" in summary
        assert "precompiled" in summary
        assert "Fallback" in summary


class TestBackwardCompatibility:
    """Test backward compatibility with existing configurations."""

    def test_existing_kernel_selections_work(self) -> None:
        """Test that existing kernel selections continue to work."""
        registry = KernelRegistry()

        # These should all work as before
        assert registry.get_variant("linux-lts") is not None
        assert registry.get_variant("linux") is not None
        assert registry.get_variant("linux-zen") is not None

    def test_precompiled_now_available_for_all_kernels(self) -> None:
        """Test that precompiled ZFS is now available for all default kernels."""
        registry = KernelRegistry()

        for kernel_name in ["linux-lts", "linux", "linux-zen"]:
            variant = registry.get_variant(kernel_name)
            assert variant is not None
            assert variant.supports_precompiled is True
            assert variant.zfs_precompiled_package is not None

    def test_fallback_maintains_kernel_consistency(self) -> None:
        """Test that fallback maintains kernel consistency."""
        registry = KernelRegistry()

        for kernel_name in ["linux-lts", "linux", "linux-zen"]:
            variant = registry.get_variant(kernel_name)
            assert variant is not None  # Ensure variant exists
            chain = FallbackStrategy.get_fallback_chain(variant, ZFSModuleMode.PRECOMPILED)

            # All attempts should use the same kernel variant
            for attempt_variant, _ in chain:
                assert attempt_variant.name == kernel_name


if __name__ == "__main__":
    pytest.main([__file__])
