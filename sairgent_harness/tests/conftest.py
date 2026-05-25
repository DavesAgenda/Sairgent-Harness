"""Shared fixtures for sairgent_harness unit tests."""
import sys
import os

# Ensure the harness root is on the path so imports work without installation
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
