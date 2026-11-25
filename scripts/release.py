"""Compatibility entry point for the Kaixin release command."""

from pathlib import Path
import subprocess
import sys


if __name__ == "__main__":
    try:
        raise SystemExit(
            subprocess.call(
                ["cargo", "xtask", "release", *sys.argv[1:]],
                cwd=Path(__file__).resolve().parents[1],
            )
        )
    except KeyboardInterrupt:
        raise SystemExit(130) from None
