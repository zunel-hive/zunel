"""Shared fixtures for the zunel test suite.

Hermetic-test invariants enforced here:

1. **No credential env vars.** All provider/credential-shaped env vars
   (ending in _API_KEY, _TOKEN, _SECRET, _PASSWORD, _CREDENTIALS, etc.)
   are unset before every test. Local developer keys cannot leak in.
2. **Isolated ZUNEL_HOME.** ZUNEL_HOME points to a per-test tempdir so
   code reading ``~/.zunel/*`` via ``get_zunel_home()`` (Phase 3+) cannot
   see the real one. Until ``get_zunel_home()`` lands, this is a no-op
   for production code but still protects against shell pollution.
3. **Deterministic runtime.** TZ=UTC, LANG=C.UTF-8, PYTHONHASHSEED=0.
4. **No ZUNEL_SESSION_* / ZUNEL_PROFILE inheritance.**

These invariants make the local test run match CI closely.
"""

from __future__ import annotations

import asyncio
import os
import signal
import sys
import warnings
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))


# Any env var ending in one of these suffixes is unset for every test.
_CREDENTIAL_SUFFIXES = (
    "_API_KEY",
    "_TOKEN",
    "_SECRET",
    "_PASSWORD",
    "_CREDENTIALS",
    "_ACCESS_KEY",
    "_SECRET_ACCESS_KEY",
    "_PRIVATE_KEY",
    "_OAUTH_TOKEN",
    "_WEBHOOK_SECRET",
    "_CLIENT_SECRET",
    "_APP_SECRET",
)

_CREDENTIAL_NAMES = frozenset({
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "GROQ_API_KEY",
    "MISTRAL_API_KEY",
    "DEEPSEEK_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "XAI_API_KEY",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "SLACK_BOT_TOKEN",
    "SLACK_APP_TOKEN",
    "SLACK_USER_TOKEN",
    "SLACK_CLIENT_ID",
    "SLACK_CLIENT_SECRET",
    "SLACK_SIGNING_SECRET",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "LANGSMITH_API_KEY",
})


# ZUNEL_* vars that change behavior by being set. Unset all of these
# unconditionally; individual tests that need them set do so explicitly.
_ZUNEL_BEHAVIORAL_VARS = frozenset({
    "ZUNEL_HOME",
    "ZUNEL_PROFILE",
    "ZUNEL_QUIET",
    "ZUNEL_INTERACTIVE",
    "ZUNEL_DEV",
    "ZUNEL_CONFIG",
    "ZUNEL_WORKSPACE",
    "ZUNEL_SESSION_ID",
    "ZUNEL_SESSION_KEY",
    "ZUNEL_SESSION_PLATFORM",
    "ZUNEL_SESSION_CHANNEL",
    "ZUNEL_PLATFORM",
})


def _looks_like_credential(name: str) -> bool:
    """True if env var name matches a credential-shaped pattern."""
    if name in _CREDENTIAL_NAMES:
        return True
    return any(name.endswith(suf) for suf in _CREDENTIAL_SUFFIXES)


@pytest.fixture(autouse=True)
def _hermetic_environment(tmp_path_factory, monkeypatch):
    """Blank credential / behavioral env vars and isolate ZUNEL_HOME.

    Pins TZ/LANG/PYTHONHASHSEED for deterministic locale + datetime tests.

    Uses ``tmp_path_factory`` (not ``tmp_path``) so the synthetic
    ZUNEL_HOME lives outside the test's own ``tmp_path``. Tests that list
    or count entries in their own ``tmp_path`` shouldn't see this dir.
    """
    for name in list(os.environ.keys()):
        if _looks_like_credential(name):
            monkeypatch.delenv(name, raising=False)

    for name in _ZUNEL_BEHAVIORAL_VARS:
        monkeypatch.delenv(name, raising=False)

    # Redirect ZUNEL_HOME to a per-test tempdir. No-op for code that still
    # uses ``Path.home() / ".zunel"`` directly; once Phase 3 lands and
    # callers switch to ``get_zunel_home()``, this becomes load-bearing.
    fake_zunel_home = tmp_path_factory.mktemp("zunel_home")
    monkeypatch.setenv("ZUNEL_HOME", str(fake_zunel_home))

    monkeypatch.setenv("TZ", "UTC")
    monkeypatch.setenv("LANG", "C.UTF-8")
    monkeypatch.setenv("LC_ALL", "C.UTF-8")
    monkeypatch.setenv("PYTHONHASHSEED", "0")


@pytest.fixture(autouse=True)
def _ensure_current_event_loop(request):
    """Provide a default event loop for sync tests calling get_event_loop().

    Python 3.11+ no longer guarantees a current loop for plain synchronous
    tests. Some tests still use ``asyncio.get_event_loop().run_until_complete()``;
    ensure they always have a usable loop without interfering with
    pytest-asyncio's own loop management for ``@pytest.mark.asyncio`` tests.
    """
    if request.node.get_closest_marker("asyncio") is not None:
        yield
        return

    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            loop = asyncio.get_event_loop_policy().get_event_loop()
    except RuntimeError:
        loop = None

    created = loop is None or loop.is_closed()
    if created:
        loop = asyncio.new_event_loop()
        asyncio.set_event_loop(loop)

    try:
        yield
    finally:
        if created and loop is not None:
            try:
                loop.close()
            finally:
                asyncio.set_event_loop(None)


def _timeout_handler(signum, frame):
    raise TimeoutError("Test exceeded 30 second timeout")


@pytest.fixture(autouse=True)
def _enforce_test_timeout():
    """Kill any individual test that takes longer than 30 seconds.

    SIGALRM is Unix-only; skip on Windows. Prevents hanging tests
    (subprocess spawns, blocking I/O) from stalling the entire suite.
    """
    if sys.platform == "win32":
        yield
        return
    old = signal.signal(signal.SIGALRM, _timeout_handler)
    signal.alarm(30)
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, old)
