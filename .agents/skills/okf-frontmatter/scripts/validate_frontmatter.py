#!/usr/bin/env python3
"""Pass 3 wrapper: validate finished OKF concept files on disk. No LLM."""

from __future__ import annotations

import runpy
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.argv = [str(HERE / "extract_frontmatter.py"), "--validate", *sys.argv[1:]]
runpy.run_path(str(HERE / "extract_frontmatter.py"), run_name="__main__")
