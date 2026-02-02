# Clean sys.argv before archinstall import to prevent argparse conflicts
import sys

sys.argv = [sys.argv[0]]
