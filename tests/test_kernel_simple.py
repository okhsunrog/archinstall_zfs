"""
Tests for the simplified kernel management system.

This test suite validates the simple kernel configuration and installation logic.
"""

from unittest.mock import Mock, patch

import pytest
from archinstall.lib.exceptions import SysCallError
from archinstall_zfs.kernel import (
    AVAILABLE_KERNELS,
    get_kernel_display_name,
    get_kernel_info,
    get_menu_options,
    get_supported_kernels,
    get_zfs_packages_for_kernel,
    install_zfs_packages,
    install_zfs_with_fallback,
    supports_precompiled_zfs,
    validate_kernel_zfs_plan,
)
from archinstall_zfs.shared import ZFSModuleMode


class TestKernelInfo:
    """Test kernel information functions."""

    def test_get_supported_kernels(self) -> None:
        """Test getting list of supported kernels."""
        kernels = get_supported_kernels()
        assert "linux-lts" in kernels
        assert "linux" in kernels
        assert "linux-zen" in kernels
        assert "linux-hardened" in kernels

    def test_get_kernel_info_valid(self) -> None:
        """Test getting kernel info for valid kernels."""
        lts_info = get_kernel_info("linux-lts")
        assert lts_info.name == "linux-lts"
        assert lts_info.display_name == "Linux LTS"
        assert lts_info.precompiled_package == "zfs-linux-lts"
        assert lts_info.headers_package == "linux-lts-headers"

    def test_get_kernel_info_invalid(self) -> None:
        """Test getting kernel info for unsupported kernel."""
        with pytest.raises(ValueError, match="Unsupported kernel: linux-invalid"):
            get_kernel_info("linux-invalid")

    def test_get_kernel_display_name(self) -> None:
        """Test getting human-readable kernel names."""
        assert get_kernel_display_name("linux-lts") == "Linux LTS"
        assert get_kernel_display_name("linux-zen") == "Linux Zen"

    def test_supports_precompiled_zfs(self) -> None:
        """Test checking precompiled ZFS support."""
        # All current kernels support precompiled ZFS
        for kernel in get_supported_kernels():
            assert supports_precompiled_zfs(kernel) is True


class TestPackageSelection:
    """Test package selection logic."""

    def test_get_zfs_packages_precompiled(self) -> None:
        """Test getting packages for precompiled mode."""
        packages = get_zfs_packages_for_kernel("linux-lts", ZFSModuleMode.PRECOMPILED)
        assert packages == ["zfs-utils", "zfs-linux-lts"]

        packages = get_zfs_packages_for_kernel("linux-zen", ZFSModuleMode.PRECOMPILED)
        assert packages == ["zfs-utils", "zfs-linux-zen"]

    def test_get_zfs_packages_dkms(self) -> None:
        """Test getting packages for DKMS mode."""
        packages = get_zfs_packages_for_kernel("linux-lts", ZFSModuleMode.DKMS)
        assert packages == ["zfs-utils", "zfs-dkms", "linux-lts-headers"]

        packages = get_zfs_packages_for_kernel("linux-hardened", ZFSModuleMode.DKMS)
        assert packages == ["zfs-utils", "zfs-dkms", "linux-hardened-headers"]

    def test_get_zfs_packages_precompiled_unsupported(self) -> None:
        """Test getting packages for unsupported precompiled."""
        # Create a mock kernel without precompiled support
        with (
            patch.dict(
                AVAILABLE_KERNELS,
                {"linux-test": type("obj", (), {"name": "linux-test", "precompiled_package": None, "headers_package": "linux-test-headers"})()},
            ),
            pytest.raises(ValueError, match="does not support precompiled ZFS"),
        ):
            get_zfs_packages_for_kernel("linux-test", ZFSModuleMode.PRECOMPILED)


class TestPackageInstallation:
    """Test package installation functions."""

    @patch("archinstall_zfs.kernel.SysCommand")
    def test_install_zfs_packages_host_success(self, mock_syscmd: Mock) -> None:
        """Test successful host package installation."""
        result = install_zfs_packages("linux-lts", ZFSModuleMode.PRECOMPILED, None)
        assert result is True
        mock_syscmd.assert_called_once_with("pacman -S --noconfirm zfs-utils zfs-linux-lts")

    def test_install_zfs_packages_target_success(self) -> None:
        """Test successful target package installation."""
        mock_installation = Mock()
        result = install_zfs_packages("linux-zen", ZFSModuleMode.DKMS, mock_installation)
        assert result is True
        mock_installation.arch_chroot.assert_called_once_with("pacman -S --noconfirm zfs-utils zfs-dkms linux-zen-headers")

    @patch("archinstall_zfs.kernel.SysCommand")
    def test_install_zfs_packages_failure(self, mock_syscmd: Mock) -> None:
        """Test package installation failure."""
        mock_syscmd.side_effect = SysCallError("Installation failed")
        result = install_zfs_packages("linux-lts", ZFSModuleMode.PRECOMPILED, None)
        assert result is False


class TestFallbackLogic:
    """Test fallback installation logic."""

    @patch("archinstall_zfs.kernel.install_zfs_packages")
    def test_install_with_fallback_precompiled_success(self, mock_install: Mock) -> None:
        """Test successful precompiled installation."""
        mock_install.return_value = True
        success, mode = install_zfs_with_fallback("linux-lts", ZFSModuleMode.PRECOMPILED, None)
        assert success is True
        assert mode == ZFSModuleMode.PRECOMPILED
        mock_install.assert_called_once_with("linux-lts", ZFSModuleMode.PRECOMPILED, None)

    @patch("archinstall_zfs.kernel.install_zfs_packages")
    def test_install_with_fallback_precompiled_fails_dkms_succeeds(self, mock_install: Mock) -> None:
        """Test precompiled fails, DKMS succeeds."""
        # First call (precompiled) fails, second call (DKMS) succeeds
        mock_install.side_effect = [False, True]
        success, mode = install_zfs_with_fallback("linux-zen", ZFSModuleMode.PRECOMPILED, None)
        assert success is True
        assert mode == ZFSModuleMode.DKMS
        assert mock_install.call_count == 2

    @patch("archinstall_zfs.kernel.install_zfs_packages")
    def test_install_with_fallback_dkms_preferred(self, mock_install: Mock) -> None:
        """Test DKMS preferred mode."""
        mock_install.return_value = True
        success, mode = install_zfs_with_fallback("linux-lts", ZFSModuleMode.DKMS, None)
        assert success is True
        assert mode == ZFSModuleMode.DKMS
        mock_install.assert_called_once_with("linux-lts", ZFSModuleMode.DKMS, None)

    @patch("archinstall_zfs.kernel.install_zfs_packages")
    def test_install_with_fallback_all_fail(self, mock_install: Mock) -> None:
        """Test all installation methods fail."""
        mock_install.return_value = False
        success, mode = install_zfs_with_fallback("linux-zen", ZFSModuleMode.PRECOMPILED, None)
        assert success is False
        assert mode == ZFSModuleMode.PRECOMPILED  # Returns original mode


class TestValidation:
    """Test validation functions."""

    def test_validate_kernel_zfs_plan_valid(self) -> None:
        """Test validation of valid plans."""
        warnings = validate_kernel_zfs_plan("linux-lts", ZFSModuleMode.PRECOMPILED)
        assert warnings == []

        warnings = validate_kernel_zfs_plan("linux-zen", ZFSModuleMode.DKMS)
        assert warnings == []

    def test_validate_kernel_zfs_plan_invalid_kernel(self) -> None:
        """Test validation with invalid kernel."""
        warnings = validate_kernel_zfs_plan("linux-invalid", ZFSModuleMode.PRECOMPILED)
        assert len(warnings) == 1
        assert "Unsupported kernel" in warnings[0]

    def test_validate_kernel_zfs_plan_no_precompiled(self) -> None:
        """Test validation when precompiled not available."""
        # Create a mock kernel without precompiled support
        with patch.dict(
            AVAILABLE_KERNELS, {"linux-test": type("obj", (), {"name": "linux-test", "precompiled_package": None, "headers_package": "linux-test-headers"})()}
        ):
            warnings = validate_kernel_zfs_plan("linux-test", ZFSModuleMode.PRECOMPILED)
            assert len(warnings) == 1
            assert "will use DKMS" in warnings[0]


class TestMenuOptions:
    """Test menu option generation."""

    @patch("archinstall_zfs.validation.should_filter_kernel_options")
    def test_get_menu_options_no_filtering(self, mock_should_filter: Mock) -> None:
        """Test menu option generation without filtering."""
        mock_should_filter.return_value = False
        options, filtered_kernels = get_menu_options()

        # Should have options for all kernels
        kernel_names = [opt[1] for opt in options]
        assert "linux-lts" in kernel_names
        assert "linux" in kernel_names
        assert "linux-zen" in kernel_names
        assert "linux-hardened" in kernel_names

        # Should have both precompiled and DKMS for each kernel
        lts_options = [opt for opt in options if opt[1] == "linux-lts"]
        assert len(lts_options) == 2  # precompiled + DKMS

        # Check that recommended option is marked
        lts_precompiled = next(opt for opt in options if opt[1] == "linux-lts" and opt[2] == ZFSModuleMode.PRECOMPILED)
        assert "recommended" in lts_precompiled[0]

        # No kernels should be filtered
        assert filtered_kernels == []

    @patch("archinstall_zfs.validation.get_compatible_kernels")
    @patch("archinstall_zfs.validation.should_filter_kernel_options")
    def test_get_menu_options_with_filtering(self, mock_should_filter: Mock, mock_get_compatible: Mock) -> None:
        """Test menu option generation with filtering."""
        mock_should_filter.return_value = True
        mock_get_compatible.return_value = (["linux-lts", "linux"], ["linux-zen"])
        
        options, filtered_kernels = get_menu_options()

        # Should have precompiled options for all kernels (precompiled is always compatible)
        precompiled_kernels = [opt[1] for opt in options if opt[2] == ZFSModuleMode.PRECOMPILED]
        assert "linux-lts" in precompiled_kernels
        assert "linux" in precompiled_kernels
        assert "linux-zen" in precompiled_kernels

        # Should only have DKMS options for compatible kernels
        dkms_kernels = [opt[1] for opt in options if opt[2] == ZFSModuleMode.DKMS]
        assert "linux-lts" in dkms_kernels
        assert "linux" in dkms_kernels
        assert "linux-zen" not in dkms_kernels  # Filtered out

        # Should report filtered kernels
        assert "Linux Zen" in filtered_kernels
