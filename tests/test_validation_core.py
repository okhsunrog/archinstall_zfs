"""
Tests for validation_core module - the core validation logic.
"""

from unittest.mock import Mock, patch

import validation_core


class TestVersionParsing:
    """Test version parsing functionality."""

    def test_parse_version_basic(self) -> None:
        """Test basic version parsing."""
        assert validation_core.parse_version("6.15") == (6, 15, 0)
        assert validation_core.parse_version("6.15.9") == (6, 15, 0)  # Normalized for kernel compat
        assert validation_core.parse_version("4.18") == (4, 18, 0)

    def test_parse_version_kernel_suffixes(self) -> None:
        """Test parsing kernel versions with suffixes."""
        assert validation_core.parse_version("6.15.9.hardened1") == (6, 15, 0)
        assert validation_core.parse_version("6.16.arch2") == (6, 16, 0)
        assert validation_core.parse_version("6.12.41-2") == (6, 12, 0)

    def test_parse_version_edge_cases(self) -> None:
        """Test edge cases in version parsing."""
        assert validation_core.parse_version("6") == (6, 0, 0)
        assert validation_core.parse_version("invalid") == (0, 0, 0)
        assert validation_core.parse_version("") == (0, 0, 0)


class TestPrecompiledCompatibility:
    """Test precompiled ZFS compatibility validation."""

    @patch("validation_core.get_package_version")
    def test_precompiled_exact_match_compatible(self, mock_get_version: Mock) -> None:
        """Test precompiled compatibility with exact version match."""
        mock_get_version.side_effect = lambda pkg: {"linux-lts": "6.12.41-2", "zfs-linux-lts": "2.3.3_6.12.41-2"}.get(pkg)

        is_compatible, warnings = validation_core.validate_precompiled_zfs_compatibility("linux-lts")

        assert is_compatible is True
        assert len(warnings) == 0

    @patch("validation_core.get_package_version")
    def test_precompiled_version_mismatch_incompatible(self, mock_get_version: Mock) -> None:
        """Test precompiled incompatibility with version mismatch."""
        mock_get_version.side_effect = lambda pkg: {"linux-zen": "6.16.zen2-1", "zfs-linux-zen": "2.3.3_6.15.9.zen1.1-1"}.get(pkg)

        is_compatible, warnings = validation_core.validate_precompiled_zfs_compatibility("linux-zen")

        assert is_compatible is False
        assert len(warnings) == 1
        assert "does not match precompiled ZFS" in warnings[0]

    @patch("validation_core.get_package_version")
    def test_precompiled_missing_package(self, mock_get_version: Mock) -> None:
        """Test handling of missing precompiled package."""
        mock_get_version.side_effect = lambda pkg: {
            "linux-lts": "6.12.41-2",
            "zfs-linux-lts": None,  # Package not found
        }.get(pkg)

        is_compatible, warnings = validation_core.validate_precompiled_zfs_compatibility("linux-lts")

        assert is_compatible is False
        assert any("Could not determine" in w for w in warnings)

    @patch("validation_core.get_package_version")
    def test_precompiled_zfs_build_suffix_match(self, mock_get_version: Mock) -> None:
        """Test precompiled compatibility with ZFS build suffix (.1, .2, etc.)."""
        # Real-world case: kernel 6.16.4.arch1-1 should match ZFS 2.3.4_6.16.4.arch1.1-1
        mock_get_version.side_effect = lambda pkg: {"linux": "6.16.4.arch1-1", "zfs-linux": "2.3.4_6.16.4.arch1.1-1"}.get(pkg)

        is_compatible, warnings = validation_core.validate_precompiled_zfs_compatibility("linux")

        assert is_compatible is True
        assert len(warnings) == 0


class TestDKMSCompatibility:
    """Test DKMS ZFS compatibility validation."""

    @patch("validation_core.fetch_zfs_kernel_compatibility")
    @patch("validation_core.get_package_version")
    def test_dkms_compatible_kernel(self, mock_get_version: Mock, mock_fetch_compat: Mock) -> None:
        """Test DKMS compatibility with supported kernel."""
        mock_get_version.side_effect = lambda pkg: {"linux-lts": "6.12.41-2", "zfs-dkms": "2.3.3-1"}.get(pkg)
        mock_fetch_compat.return_value = ("4.18", "6.15")

        is_compatible, warnings = validation_core.validate_kernel_zfs_compatibility("linux-lts", "dkms")

        assert is_compatible is True
        assert len(warnings) == 0

    @patch("validation_core.fetch_zfs_kernel_compatibility")
    @patch("validation_core.get_package_version")
    def test_dkms_incompatible_kernel(self, mock_get_version: Mock, mock_fetch_compat: Mock) -> None:
        """Test DKMS incompatibility with unsupported kernel."""
        mock_get_version.side_effect = lambda pkg: {"linux": "6.16.arch2-1", "zfs-dkms": "2.3.3-1"}.get(pkg)
        mock_fetch_compat.return_value = ("4.18", "6.15")

        is_compatible, warnings = validation_core.validate_kernel_zfs_compatibility("linux", "dkms")

        assert is_compatible is False
        assert len(warnings) == 1
        assert "outside the supported range" in warnings[0]

    @patch("validation_core.fetch_zfs_kernel_compatibility")
    @patch("validation_core.get_package_version")
    def test_dkms_missing_compatibility_data(self, mock_get_version: Mock, mock_fetch_compat: Mock) -> None:
        """Test DKMS validation when compatibility data unavailable."""
        mock_get_version.side_effect = lambda pkg: {"linux-lts": "6.12.41-2", "zfs-dkms": "2.3.3-1"}.get(pkg)
        mock_fetch_compat.return_value = None  # API failure

        is_compatible, warnings = validation_core.validate_kernel_zfs_compatibility("linux-lts", "dkms")

        assert is_compatible is False
        assert any("Could not fetch ZFS kernel compatibility" in w for w in warnings)


class TestKernelCompatibilityRanges:
    """Test that kernel compatibility ranges work correctly."""

    def test_kernel_range_boundaries(self) -> None:
        """Test version comparisons at range boundaries."""
        # Test versions that should be compatible with 4.18 - 6.15 range
        compatible_versions = [
            "4.18",
            "4.19",
            "5.0",
            "6.12.41",
            "6.15",
            "6.15.9.hardened1",  # This was the bug we fixed
        ]

        # Test versions that should be incompatible
        incompatible_versions = [
            "4.17",
            "6.16",
            "6.16.arch2",
            "7.0",
        ]

        min_version = validation_core.parse_version("4.18")
        max_version = validation_core.parse_version("6.15")

        for version_str in compatible_versions:
            version = validation_core.parse_version(version_str)
            assert min_version <= version <= max_version, f"{version_str} should be compatible"

        for version_str in incompatible_versions:
            version = validation_core.parse_version(version_str)
            assert not (min_version <= version <= max_version), f"{version_str} should be incompatible"
