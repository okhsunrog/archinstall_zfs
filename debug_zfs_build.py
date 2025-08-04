#!/usr/bin/env python3
"""
Debug wrapper for ZFS package building to get better error visibility
"""

import subprocess
import sys
import os
from pathlib import Path

def main():
    print("=== Debug ZFS Package Builder ===")
    print(f"Working directory: {os.getcwd()}")
    print(f"Python executable: {sys.executable}")
    print(f"Python version: {sys.version}")
    
    # Check if we're running as root (makepkg won't work)
    if os.geteuid() == 0:
        print("❌ ERROR: Running as root! makepkg requires non-root user.")
        print("Please run as a regular user with sudo access.")
        return 1
    
    # Check basic dependencies
    print("\n=== Checking dependencies ===")
    deps = ['git', 'makepkg', 'pacman', 'python3']
    for dep in deps:
        try:
            result = subprocess.run(['which', dep], capture_output=True, text=True)
            if result.returncode == 0:
                print(f"✅ {dep}: {result.stdout.strip()}")
            else:
                print(f"❌ {dep}: not found")
        except Exception as e:
            print(f"❌ {dep}: error checking - {e}")
    
    # Check if we have the script
    script_path = Path("gen_iso/build_zfs_package.py")
    if not script_path.exists():
        print(f"❌ Script not found: {script_path}")
        return 1
    
    print(f"✅ Script found: {script_path}")
    
    # Run with maximum verbosity and error capture
    print("\n=== Running ZFS package builder ===")
    try:
        # Set environment for maximum debugging
        env = os.environ.copy()
        env['PYTHONUNBUFFERED'] = '1'
        env['ZFS_BUILDER_DEBUG'] = '1'
        
        # Run the script with real-time output
        process = subprocess.Popen(
            [sys.executable, str(script_path)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            universal_newlines=True,
            env=env
        )
        
        # Stream output in real-time
        if process.stdout:
            while True:
                line = process.stdout.readline()
                if line:
                    print(line.rstrip())
                elif process.poll() is not None:
                    break
        
        return_code = process.wait()
        print(f"\n=== Process completed with exit code: {return_code} ===")
        
        if return_code != 0:
            print("❌ Build failed!")
            
            # Check for artifacts directory
            artifacts_dir = Path("artifacts")
            if artifacts_dir.exists():
                print(f"\n=== Artifacts found in {artifacts_dir} ===")
                for log_file in artifacts_dir.glob("*.log"):
                    print(f"📄 {log_file.name}:")
                    try:
                        content = log_file.read_text()
                        # Show last 50 lines of each log
                        lines = content.splitlines()
                        if len(lines) > 50:
                            print("... (truncated) ...")
                            lines = lines[-50:]
                        for line in lines:
                            print(f"  {line}")
                        print()
                    except Exception as e:
                        print(f"  Error reading log: {e}")
            else:
                print("No artifacts directory found")
        
        return return_code
        
    except KeyboardInterrupt:
        print("\n❌ Interrupted by user")
        return 130
    except Exception as e:
        print(f"❌ Unexpected error: {e}")
        return 1

if __name__ == "__main__":
    sys.exit(main())