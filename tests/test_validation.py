"""
Tests for the kernel validation system.

This test suite validates the essential behavior of the DKMS compatibility checking.
"""

from unittest.mock import Mock, patch

from archinstall_zfs.validation import (
    get_compatible_kernels,
    should_filter_kernel_options,
    validate_kernel_zfs_compatibility,
)


class TestValidationBehavior:
    """Test core validation behavior that users depend on."""

    def test_validate_precompiled_always_compatible(self) -> None:
        """Test that precompiled mode is always considered compatible."""
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "precompiled")
        assert is_compatible is True
        assert len(warnings) == 0

    @patch("archinstall_zfs.validation._core_validate_kernel_zfs_compatibility")
    def test_validate_dkms_compatible(self, mock_validate: Mock) -> None:
        """Test DKMS validation when kernel is compatible."""
        mock_validate.return_value = (True, [])
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-lts", "dkms")
        assert is_compatible is True
        assert len(warnings) == 0

    @patch("archinstall_zfs.validation._core_validate_kernel_zfs_compatibility")
    def test_validate_dkms_incompatible(self, mock_validate: Mock) -> None:
        """Test DKMS validation when kernel is incompatible."""
        mock_validate.return_value = (False, ["Kernel too new"])
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is False
        assert len(warnings) == 1

    @patch("archinstall_zfs.validation._core_validate_kernel_zfs_compatibility")
    def test_validate_dkms_with_warnings(self, mock_validate: Mock) -> None:
        """Test DKMS validation with warnings but still compatible."""
        mock_validate.return_value = (True, ["Network issue, assuming compatible"])
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux", "dkms")
        assert is_compatible is True
        assert len(warnings) == 1


class TestKernelFiltering:
    """Test kernel filtering behavior."""

    @patch("archinstall_zfs.validation._core_get_compatible_kernels")
    def test_get_compatible_kernels_mixed_results(self, mock_get_compatible: Mock) -> None:
        """Test filtering with some compatible and some incompatible kernels."""
        mock_get_compatible.return_value = (["linux-lts", "linux"], ["linux-zen"])
        available_kernels = ["linux-lts", "linux", "linux-zen"]
        compatible, incompatible = get_compatible_kernels(available_kernels)

        assert "linux-lts" in compatible
        assert "linux" in compatible
        assert "linux-zen" in incompatible


class TestFilteringConfiguration:
    """Test filtering configuration."""

    def test_should_filter_kernel_options_default(self) -> None:
        """Test default filtering behavior (enabled)."""
        assert should_filter_kernel_options() is True

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_FILTER_KERNELS": "0"})
    def test_should_filter_kernel_options_disabled(self) -> None:
        """Test filtering disabled by environment variable."""
        assert should_filter_kernel_options() is False

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_FILTER_KERNELS": "false"})
    def test_should_filter_kernel_options_disabled_false(self) -> None:
        """Test filtering disabled with 'false'."""
        assert should_filter_kernel_options() is False

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_FILTER_KERNELS": "1"})
    def test_should_filter_kernel_options_enabled_explicit(self) -> None:
        """Test filtering explicitly enabled."""
        assert should_filter_kernel_options() is True
