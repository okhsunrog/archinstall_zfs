"""
Tests for the kernel validation system.

This test suite validates the proactive DKMS compatibility checking functionality.
"""

import json
from unittest.mock import Mock, patch

from archinstall_zfs.validation import (
    fetch_zfs_kernel_compatibility,
    get_compatible_kernels,
    get_package_version,
    should_filter_kernel_options,
    validate_kernel_zfs_compatibility,
)


class TestPackageVersion:
    """Test package version querying."""

    @patch("archinstall_zfs.validation.SysCommand")
    def test_get_package_version_success(self, mock_syscmd: Mock) -> None:
        """Test successful package version retrieval."""
        mock_syscmd.return_value.decode.return_value = "Version        : 2.3.3-1"
        version = get_package_version("zfs-dkms")
        assert version == "2.3.3-1"
        mock_syscmd.assert_called_once_with("pacman -Si zfs-dkms")

    @patch("archinstall_zfs.validation.SysCommand")
    def test_get_package_version_failure(self, mock_syscmd: Mock) -> None:
        """Test package version retrieval failure."""
        mock_syscmd.side_effect = Exception("Package not found")
        version = get_package_version("nonexistent-package")
        assert version is None

    @patch("archinstall_zfs.validation.SysCommand")
    def test_get_package_version_no_version_line(self, mock_syscmd: Mock) -> None:
        """Test package query with no version line."""
        mock_syscmd.return_value.decode.return_value = "Name        : zfs-dkms\nDescription : ZFS DKMS"
        version = get_package_version("zfs-dkms")
        assert version is None


class TestZFSCompatibility:
    """Test ZFS compatibility fetching."""

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_success(self, mock_syscmd: Mock) -> None:
        """Test successful compatibility fetching."""
        mock_response = {"body": "## Features\n\nLinux: compatible with 6.1 - 6.11 kernels\n\n## Bug Fixes"}
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)

        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result == ("6.1", "6.11")

        # Check API URL construction
        expected_url = "https://api.github.com/repos/openzfs/zfs/releases/tags/zfs-2.3.3"
        mock_syscmd.assert_called_once()
        call_args = mock_syscmd.call_args[0][0]
        assert expected_url in call_args

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_real_zfs_233(self, mock_syscmd: Mock) -> None:
        """Test with actual ZFS 2.3.3 release body format."""
        # This is based on the actual ZFS 2.3.3 release notes format from GitHub API
        mock_response = {
            "body": (
                "#### Supported Platforms\n"
                "- **Linux**: compatible with 4.18 - 6.15 kernels\n"
                "- **FreeBSD**: compatible with releases starting from 13.3+, 14.0+\n\n"
                "#### Changes\n"
                "- Tag zfs-2.3.3\n"
                "- Linux 6.15 compat: META #17393\n"
                "- Fix mixed-use-of-spaces-and-tabs rpmlint warning #17461\n"
            )
        }
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)

        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result == ("4.18", "6.15")

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_real_zfs_227(self, mock_syscmd: Mock) -> None:
        """Test with actual ZFS 2.2.7 release body format."""
        # This is based on the actual ZFS 2.2.7 release notes format from GitHub API
        mock_response = {
            "body": (
                "#### Supported Platforms\n"
                "- **Linux**: compatible with 4.18 - 6.12 kernels\n"
                "- **FreeBSD**: compatible with releases starting from 13.0-RELEASE\n\n"
                "#### Changes\n"
                "- add get_name implementation for exports. (#16833)\n"
                "- Fix race in libzfs_run_process_impl #16801\n"
                "- Linux: Fix detection of register_sysctl_sz\n"
                "- Linux: Fix zfs_prune panics #16770\n"
                "- Linux 6.12 compat: META #16793\n"
            )
        }
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)

        result = fetch_zfs_kernel_compatibility("2.2.7-1")
        assert result == ("4.18", "6.12")

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_network_failure(self, mock_syscmd: Mock) -> None:
        """Test compatibility fetching with network failure."""
        mock_syscmd.side_effect = Exception("Network error")
        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result is None

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_api_error(self, mock_syscmd: Mock) -> None:
        """Test compatibility fetching with API error."""
        mock_response = {"message": "Not Found"}
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)
        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result is None

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_no_body(self, mock_syscmd: Mock) -> None:
        """Test compatibility fetching with empty release body."""
        mock_response = {"body": ""}
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)
        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result is None

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_no_pattern_match(self, mock_syscmd: Mock) -> None:
        """Test compatibility fetching with no matching pattern."""
        mock_response = {"body": "## Features\n\nSome other changes\n\n## Bug Fixes"}
        mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)
        result = fetch_zfs_kernel_compatibility("2.3.3-1")
        assert result is None

    @patch("archinstall_zfs.validation.SysCommand")
    def test_fetch_zfs_kernel_compatibility_alternative_patterns(self, mock_syscmd: Mock) -> None:
        """Test compatibility fetching with alternative patterns."""
        test_cases = [
            ("Kernel compatibility: 6.0 - 6.10", ("6.0", "6.10")),
            ("Linux kernel 6.2 - 6.12 support", ("6.2", "6.12")),
        ]

        for body_text, expected in test_cases:
            mock_response = {"body": body_text}
            mock_syscmd.return_value.decode.return_value = json.dumps(mock_response)
            result = fetch_zfs_kernel_compatibility("2.3.3-1")
            assert result == expected


class TestCompatibilityValidation:
    """Test compatibility validation logic."""

    def test_validate_kernel_zfs_compatibility_precompiled(self) -> None:
        """Test validation for precompiled mode (always compatible)."""
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "precompiled")
        assert is_compatible is True
        assert warnings == []

    def test_validate_kernel_zfs_compatibility_realistic_scenario(self) -> None:
        """Test a realistic scenario with ZFS 2.3.3 and various kernels."""
        with (
            patch("archinstall_zfs.validation.get_package_version") as mock_get_version,
            patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility") as mock_fetch,
        ):
            # Mock ZFS 2.3.3 supports kernels 4.18 - 6.15
            mock_fetch.return_value = ("4.18", "6.15")

            # Test compatible kernel (6.8)
            mock_get_version.side_effect = ["2.3.3-1", "6.8.1-arch1-1"]
            is_compatible, warnings = validate_kernel_zfs_compatibility("linux", "dkms")
            assert is_compatible is True
            assert warnings == []

            # Test kernel too new (6.16)
            mock_get_version.side_effect = ["2.3.3-1", "6.16.1-arch1-1"]
            is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
            assert is_compatible is False
            assert "outside the supported range" in warnings[0]
            assert "4.18 - 6.15" in warnings[0]

            # Test kernel too old (4.17)
            mock_get_version.side_effect = ["2.3.3-1", "4.17.1-arch1-1"]
            is_compatible, warnings = validate_kernel_zfs_compatibility("linux-lts", "dkms")
            assert is_compatible is False
            assert "outside the supported range" in warnings[0]

    def test_validate_kernel_zfs_compatibility_zfs_227_scenario(self) -> None:
        """Test a realistic scenario with ZFS 2.2.7 and various kernels."""
        with (
            patch("archinstall_zfs.validation.get_package_version") as mock_get_version,
            patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility") as mock_fetch,
        ):
            # Mock ZFS 2.2.7 supports kernels 4.18 - 6.12
            mock_fetch.return_value = ("4.18", "6.12")

            # Test compatible kernel (6.12)
            mock_get_version.side_effect = ["2.2.7-1", "6.12.0-arch1-1"]
            is_compatible, warnings = validate_kernel_zfs_compatibility("linux", "dkms")
            assert is_compatible is True
            assert warnings == []

            # Test kernel too new (6.13) - would fail with ZFS 2.2.7
            mock_get_version.side_effect = ["2.2.7-1", "6.13.1-arch1-1"]
            is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
            assert is_compatible is False
            assert "outside the supported range" in warnings[0]
            assert "4.18 - 6.12" in warnings[0]

            # Test edge case: 6.13 would be compatible with ZFS 2.3.3 but not 2.2.7
            # This shows why the validation is important for different ZFS versions

    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_no_zfs_version(self, mock_get_version: Mock) -> None:
        """Test validation when ZFS version cannot be determined."""
        mock_get_version.return_value = None
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is True
        assert "Could not determine zfs-dkms version" in warnings[0]

    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_no_kernel_version(self, mock_get_version: Mock) -> None:
        """Test validation when kernel version cannot be determined."""
        mock_get_version.side_effect = ["2.3.3-1", None]  # ZFS version, then kernel version
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is True
        assert "Could not determine linux-zen version" in warnings[0]

    @patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility")
    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_no_api_data(self, mock_get_version: Mock, mock_fetch: Mock) -> None:
        """Test validation when API data cannot be fetched."""
        mock_get_version.side_effect = ["2.3.3-1", "6.10.1-arch1-1"]
        mock_fetch.return_value = None
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is True
        assert "Could not fetch ZFS kernel compatibility" in warnings[0]

    @patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility")
    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_compatible(self, mock_get_version: Mock, mock_fetch: Mock) -> None:
        """Test validation with compatible kernel."""
        mock_get_version.side_effect = ["2.3.3-1", "6.8.1-arch1-1"]
        mock_fetch.return_value = ("6.1", "6.11")
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is True
        assert warnings == []

    @patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility")
    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_incompatible_too_new(self, mock_get_version: Mock, mock_fetch: Mock) -> None:
        """Test validation with kernel too new."""
        mock_get_version.side_effect = ["2.3.3-1", "6.12.1-arch1-1"]
        mock_fetch.return_value = ("6.1", "6.11")
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is False
        assert "outside the supported range" in warnings[0]

    @patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility")
    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_incompatible_too_old(self, mock_get_version: Mock, mock_fetch: Mock) -> None:
        """Test validation with kernel too old."""
        mock_get_version.side_effect = ["2.3.3-1", "5.15.1-arch1-1"]
        mock_fetch.return_value = ("6.1", "6.11")
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is False
        assert "outside the supported range" in warnings[0]

    @patch("archinstall_zfs.validation.fetch_zfs_kernel_compatibility")
    @patch("archinstall_zfs.validation.get_package_version")
    def test_validate_kernel_zfs_compatibility_version_parsing_error(self, mock_get_version: Mock, mock_fetch: Mock) -> None:
        """Test validation with version parsing error."""
        mock_get_version.side_effect = ["2.3.3-1", "6.8.arch1"]  # Should parse successfully now
        mock_fetch.return_value = ("6.1", "6.11")
        is_compatible, warnings = validate_kernel_zfs_compatibility("linux-zen", "dkms")
        assert is_compatible is True
        assert warnings == []  # No parsing errors expected


class TestKernelFiltering:
    """Test kernel filtering logic."""

    @patch("archinstall_zfs.validation.validate_kernel_zfs_compatibility")
    def test_get_compatible_kernels_all_compatible(self, mock_validate: Mock) -> None:
        """Test when all kernels are compatible."""
        mock_validate.return_value = (True, [])
        available_kernels = ["linux-lts", "linux", "linux-zen"]
        compatible, incompatible = get_compatible_kernels(available_kernels)
        assert compatible == available_kernels
        assert incompatible == []

    @patch("archinstall_zfs.validation.validate_kernel_zfs_compatibility")
    def test_get_compatible_kernels_some_incompatible(self, mock_validate: Mock) -> None:
        """Test when some kernels are incompatible."""

        def validate_side_effect(kernel: str, mode: str) -> tuple[bool, list[str]]:
            if kernel == "linux-zen":
                return (False, ["Kernel too new"])
            return (True, [])

        mock_validate.side_effect = validate_side_effect
        available_kernels = ["linux-lts", "linux", "linux-zen"]
        compatible, incompatible = get_compatible_kernels(available_kernels)
        assert compatible == ["linux-lts", "linux"]
        assert incompatible == ["linux-zen"]

    @patch("archinstall_zfs.validation.validate_kernel_zfs_compatibility")
    def test_get_compatible_kernels_with_warnings(self, mock_validate: Mock) -> None:
        """Test kernel filtering with validation warnings."""
        mock_validate.return_value = (True, ["Some warning"])
        available_kernels = ["linux-lts"]
        compatible, incompatible = get_compatible_kernels(available_kernels)
        assert compatible == ["linux-lts"]
        assert incompatible == []


class TestFilteringConfiguration:
    """Test filtering configuration."""

    def test_should_filter_kernel_options_default(self) -> None:
        """Test default filtering behavior."""
        assert should_filter_kernel_options() is True

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING": "1"})
    def test_should_filter_kernel_options_disabled_by_env(self) -> None:
        """Test filtering disabled by environment variable."""
        assert should_filter_kernel_options() is False

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING": "true"})
    def test_should_filter_kernel_options_disabled_by_env_true(self) -> None:
        """Test filtering disabled by environment variable with 'true'."""
        assert should_filter_kernel_options() is False

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING": "yes"})
    def test_should_filter_kernel_options_disabled_by_env_yes(self) -> None:
        """Test filtering disabled by environment variable with 'yes'."""
        assert should_filter_kernel_options() is False

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING": "0"})
    def test_should_filter_kernel_options_enabled_by_env_zero(self) -> None:
        """Test filtering enabled by environment variable with '0'."""
        assert should_filter_kernel_options() is True

    @patch.dict("os.environ", {"ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING": "false"})
    def test_should_filter_kernel_options_enabled_by_env_false(self) -> None:
        """Test filtering enabled by environment variable with 'false'."""
        assert should_filter_kernel_options() is True
