#!/usr/bin/env python3
"""Measure the installer's Wi-Fi against a booted test ISO, over SSH.

The adapter on the test laptop drops scans and times out on connections,
and the question is whether the driver's power saving is what does it. The
answer needs the same measurement twice — with the parameters the image
booted with, and with them toggled — which is a lot of typing to do by
hand on a live medium that forgets everything on reboot.

This runs the matrix instead: a few listings and a few cold connections per
configuration, through the installer's own calls (`azfs-wifi-probe`, which
the test ISO carries), and prints what each configuration managed.

    just wifi-matrix 10.77.77.60 JustANet 'passphrase'

Nothing here writes to the machine's disks. It reloads the wireless module
between phases, which drops the Wi-Fi connection but leaves the wired one
this runs over alone.
"""

import argparse
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass, field

PROBE = "/usr/local/bin/azfs-wifi-probe"


@dataclass
class Outcome:
    """What one configuration managed over its runs."""

    scans: list[tuple[int, bool, float]] = field(default_factory=list)
    connects: list[tuple[bool, float, str]] = field(default_factory=list)

    @property
    def scans_with_target(self) -> int:
        return sum(1 for _, found, _ in self.scans if found)

    @property
    def connects_ok(self) -> int:
        return sum(1 for ok, _, _ in self.connects if ok)


def ssh(host: str, password: str, command: str, timeout: int = 180) -> str:
    """Run one command on the test machine and return its output."""
    argv = [
        "sshpass",
        "-p",
        password,
        "ssh",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "LogLevel=ERROR",
        "-o",
        "ConnectTimeout=10",
        f"root@{host}",
        command,
    ]
    done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    return done.stdout + done.stderr


def field_of(line: str, key: str) -> str | None:
    match = re.search(rf"\b{key}=(\S+)", line)
    return match.group(1) if match else None


def run_scan(host: str, password: str, ssid: str) -> tuple[int, bool, float]:
    out = ssh(host, password, f"{PROBE} scan {shlex.quote(ssid)}")
    line = next((l for l in out.splitlines() if l.startswith("result=")), "")
    return (
        int(field_of(line, "networks") or 0),
        field_of(line, "target") == "true",
        float(field_of(line, "seconds") or 0.0),
    )


def run_connect(
    host: str, password: str, ssid: str, passphrase: str, daemon: str
) -> tuple[bool, float, str]:
    # A connection that starts from a saved profile is not the one the
    # wizard makes, so every attempt starts cold.
    if daemon == "nm":
        ssh(host, password, f"nmcli connection delete {shlex.quote(ssid)} 2>/dev/null; true")
        ssh(host, password, "nmcli device disconnect $(nmcli -t -f DEVICE,TYPE device | awk -F: '$2==\"wifi\"{print $1; exit}') 2>/dev/null; true")
    else:
        ssh(host, password, f"iwctl known-networks {shlex.quote(ssid)} forget 2>/dev/null; true")
        ssh(host, password, "iwctl station wlan0 disconnect 2>/dev/null; true")

    out = ssh(
        host,
        password,
        f"{PROBE} connect {shlex.quote(ssid)} {shlex.quote(passphrase)}",
        timeout=240,
    )
    line = next((l for l in out.splitlines() if l.startswith("result=")), "")
    return (
        field_of(line, "result") == "ok",
        float(field_of(line, "seconds") or 0.0),
        (line.split("error=", 1)[1] if "error=" in line else ""),
    )


def module_parameters(host: str, password: str, module: str) -> str:
    out = ssh(
        host,
        password,
        f"for f in /sys/module/{module}/parameters/*; do "
        f'printf "%s=%s " "$(basename $f)" "$(cat $f)"; done',
    )
    return out.strip()


def reload_module(host: str, password: str, module: str, options: str) -> bool:
    """Reload the wireless module with `options`, and say whether it came back."""
    ssh(host, password, f"modprobe -r {module} 2>&1", timeout=60)
    ssh(host, password, f"modprobe {module} {options} 2>&1", timeout=60)
    ssh(host, password, "sleep 5; true", timeout=60)
    return module in ssh(host, password, "lsmod")


def measure(
    host: str, password: str, ssid: str, passphrase: str, daemon: str, rounds: int
) -> Outcome:
    outcome = Outcome()
    for _ in range(rounds):
        outcome.scans.append(run_scan(host, password, ssid))
    if passphrase:
        for _ in range(rounds):
            outcome.connects.append(run_connect(host, password, ssid, passphrase, daemon))
    return outcome


def report(name: str, parameters: str, outcome: Outcome) -> None:
    print(f"\n── {name}")
    print(f"   {parameters}")
    listings = ", ".join(
        f"{count} networks/{seconds:.1f}s{'' if found else ' (target missing)'}"
        for count, found, seconds in outcome.scans
    )
    print(f"   scans: {outcome.scans_with_target}/{len(outcome.scans)} found the target")
    print(f"     {listings}")
    if outcome.connects:
        print(f"   connects: {outcome.connects_ok}/{len(outcome.connects)} succeeded")
        for ok, seconds, error in outcome.connects:
            print(f"     {'ok' if ok else 'failed'} in {seconds:.1f}s{'' if ok else f' — {error}'}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, help="the booted test machine")
    parser.add_argument("--ssid", required=True, help="the network to look for and join")
    parser.add_argument("--passphrase", default="", help="its passphrase; without one, only scans are measured")
    parser.add_argument("--password", default="root", help="root's SSH password on the test image")
    parser.add_argument("--module", default="rtl8723be", help="the wireless driver to reload")
    parser.add_argument("--options", default="fwlps=0 ips=0", help="the module parameters to test")
    parser.add_argument("--rounds", type=int, default=4, help="listings and connections per configuration")
    args = parser.parse_args()

    daemon = "nm" if "active" in ssh(args.host, args.password, "systemctl is-active NetworkManager") else "iwd"
    if PROBE not in ssh(args.host, args.password, f"ls {PROBE} 2>&1"):
        print(f"{args.host} has no {PROBE}: boot the image `just iso-wifi-test` builds", file=sys.stderr)
        return 1
    print(f"{args.host}: {daemon}, {args.rounds} rounds per configuration")

    as_booted = module_parameters(args.host, args.password, args.module)
    report("as booted", as_booted, measure(args.host, args.password, args.ssid, args.passphrase, daemon, args.rounds))

    if not reload_module(args.host, args.password, args.module, args.options):
        print(f"\n{args.module} did not come back after reloading; stopping here", file=sys.stderr)
        return 1
    toggled = module_parameters(args.host, args.password, args.module)
    report(f"reloaded with {args.options}", toggled, measure(args.host, args.password, args.ssid, args.passphrase, daemon, args.rounds))

    # Leave the machine as it was found.
    reload_module(args.host, args.password, args.module, "")
    print("\nmodule restored to its defaults")
    return 0


if __name__ == "__main__":
    sys.exit(main())
