"""
Entry point for running zunel as a module: python -m zunel
"""

from zunel.config.profile import apply_profile_override

# MUST run before importing zunel.cli.commands (which imports zunel.config.*
# modules that may snapshot ZUNEL_HOME at import time).
apply_profile_override()

from zunel.cli.commands import app  # noqa: E402

if __name__ == "__main__":
    app()
