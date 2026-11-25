"""Compatibility entry point for the Kaixin build command."""

from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent


def main() -> int:
    return subprocess.call(["cargo", "xtask", "build", *sys.argv[1:]], cwd=ROOT)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130) from None
