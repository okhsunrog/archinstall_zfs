from pathlib import Path

from archinstall_zfs.utils import modify_zfs_cache_mountpoints


def test_modify_zfs_cache_mountpoints():
    input_content = """zroot\tnone\ton\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-
zroot/arch0/data/home\t/mnt/home\ton\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-
zroot/arch0/root\t/mnt\tnoauto\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-"""

    expected_output = """zroot\tnone\ton\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-
zroot/arch0/data/home\t/home\ton\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-
zroot/arch0/root\t/\tnoauto\ton\ton\toff\ton\toff\ton\toff\t-\tnone\t-\t-\t-\t-\t-\t-\t-\t-"""

    result = modify_zfs_cache_mountpoints(input_content, Path("/mnt"))
    print("Result:", result)
    print("Expected:", expected_output)
    assert result == expected_output


def test_modify_zfs_cache_mountpoints_deep_paths():
    input_content = "zroot/data\t/mnt/var/lib/docker\ton\ton\ton"
    expected = "zroot/data\t/var/lib/docker\ton\ton\ton"
    result = modify_zfs_cache_mountpoints(input_content, Path("/mnt"))
    print("Result:", result)
    print("Expected:", expected)
    assert result == expected


def test_modify_zfs_cache_mountpoints_empty():
    assert modify_zfs_cache_mountpoints("", Path("/mnt")) == ""


def test_modify_zfs_cache_mountpoints_no_mountpoint():
    input_content = "zroot\tnone\ton\ton\ton"
    assert modify_zfs_cache_mountpoints(input_content, Path("/mnt")) == input_content
