"""Unprivileged regression tests: uv run python gen_iso/test_live_update.py."""

from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "profile/airootfs/usr/local/libexec/azfs-live-update"


class LiveUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "USB with spaces" / "azfs"
        self.source.parent.mkdir()
        self.destination = self.root / "azfs"
        self.destination.write_bytes(b"built-in installer")

    def run_shell(self, body, *args):
        return subprocess.run(
            ["bash", "-c", 'source "$1"; shift; ' + body, "test", str(SCRIPT), *map(str, args)],
            text=True, capture_output=True, timeout=20,
        )

    def update(self, prefix=""):
        return self.run_shell(prefix + 'install_update "$1" "$2"', self.source, self.destination)

    def assert_preserved(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.destination.read_bytes(), b"built-in installer")
        self.assertEqual(list(self.root.glob("azfs.update.*")), [])

    def test_update_is_atomic_and_works_without_usb_execute_permission(self):
        shutil.copyfile("/usr/bin/true", self.source)
        self.source.chmod(0o644)
        with self.destination.open("rb") as old_inode:
            result = self.update()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(old_inode.read(), b"built-in installer")
        self.assertEqual(self.destination.read_bytes(), self.source.read_bytes())
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o755)
        self.assertIn("sha256", result.stdout)
        self.assertEqual(list(self.root.glob("azfs.update.*")), [])

    def test_missing_or_empty_update_preserves_original(self):
        self.assert_preserved(self.update())
        self.source.touch()
        self.assert_preserved(self.update())

    def test_script_is_rejected_without_execution(self):
        marker = self.root / "executed"
        self.source.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
        self.assert_preserved(self.update())
        self.assertFalse(marker.exists())

    def test_broken_elf_preserves_original(self):
        self.source.write_bytes(b"\x7fELF" + bytes(128))
        self.assert_preserved(self.update())

    def test_symlink_is_rejected(self):
        self.source.symlink_to("/usr/bin/true")
        self.assert_preserved(self.update())

    def test_partial_copy_failure_preserves_original(self):
        shutil.copyfile("/usr/bin/true", self.source)
        self.assert_preserved(self.update('cp() { printf partial > "${@: -1}"; return 1; }; '))

    def test_rename_failure_preserves_original(self):
        shutil.copyfile("/usr/bin/true", self.source)
        self.assert_preserved(self.update('mv() { return 1; }; '))

    def test_failed_startup_probe_preserves_original(self):
        shutil.copyfile("/usr/bin/true", self.source)
        self.assert_preserved(self.update('timeout() { return 124; }; '))

    def discover(self, topology, partitions):
        # Only device enumeration is simulated; selection runs the real helper.
        return self.run_shell(
            '''fixture_topology=$1; fixture_partitions=$2
            lsblk() {
                case "${@: -1}" in
                    /dev/mapper/ventoy) printf '%s\\n' "$fixture_topology" ;;
                    /dev/sdb|/dev/nvme1n1) printf '%s\\n' "$fixture_partitions" ;;
                    *) return 1 ;;
                esac
            }
            partition_number() { printf '%s\\n' "${1: -1}"; }
            find_update_partition''', topology, partitions,
        )

    def test_follows_boot_mapping_for_sata_and_nvme(self):
        for disk, part in [("sdb", "sdb1"), ("nvme1n1", "nvme1n1p1")]:
            with self.subTest(disk=disk):
                result = self.discover(
                    f"dm /dev/mapper/ventoy\ndisk /dev/{disk}",
                    f"disk /dev/{disk}\npart /dev/{part}\npart /dev/{disk}2",
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), f"/dev/{part}")

    def test_ambiguous_or_missing_boot_disk_is_rejected(self):
        for topology in ["", "dm /dev/mapper/ventoy", "disk /dev/sdb\ndisk /dev/sdc"]:
            self.assertNotEqual(self.discover(topology, "part /dev/sdb1").returncode, 0)

    def test_missing_data_partition_is_rejected(self):
        self.assertNotEqual(self.discover("disk /dev/sdb", "part /dev/sdb2").returncode, 0)

    def test_installed_system_is_untouched(self):
        result = self.run_shell('is_live_root() { return 1; }; main')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("skipping", result.stdout)

    def mapping(self, table):
        return self.run_shell(
            '''fixture_table=$1
            dmsetup() { [[ -n "$fixture_table" ]] || return 1; printf '%s\\n' "$fixture_table"; }
            blockdev() { echo 8192; }
            lsblk() { echo 8:17; }
            mount_source /dev/sdb1''', table,
        )

    def test_ventoy_partition_mapping(self):
        result = self.mapping("0 8192 linear 8:17 0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "/dev/mapper/sdb1")

    def test_mapping_of_another_partition_or_range_is_rejected(self):
        for table in ["0 8192 linear 8:33 0", "0 4096 linear 8:17 0", "0 8192 linear 8:17 1"]:
            self.assertNotEqual(self.mapping(table).returncode, 0)

    def test_direct_partition_without_ventoy_remount_mapping(self):
        result = self.mapping("")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "/dev/sdb1")


if __name__ == "__main__":
    unittest.main()
