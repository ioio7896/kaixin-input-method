"""Run build tools with case-normalized environment names for Windows MSBuild."""
import os
import subprocess
import sys
env = {key.upper(): value for key, value in os.environ.items()}
raise SystemExit(subprocess.run(sys.argv[1:], env=env).returncode)
