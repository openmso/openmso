# SPDX-License-Identifier: Apache-2.0
"""Resolve a capture plugin name to its launch argv via plugins/<name>/plugin.json.

"Capture plugin" is the main-project packaging of an OCP CaptureServer; the
protocol library (openmso-api) has no notion of plugins or manifests.
"""

import json
import os
import sys


def find_plugin(name, repo_root=None):
    """Resolve a plugin name to its launch argv via plugins/<name>/plugin.json."""
    if repo_root is None:
        # python/plugin_manifest.py -> repo root is two levels up
        repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    manifest_path = os.path.join(repo_root, "plugins", name, "plugin.json")
    with open(manifest_path) as f:
        manifest = json.load(f)
    argv = [a.replace("{python}", sys.executable) for a in manifest["run"]]
    # Relative path entries (anything with a separator, or a *.py script)
    # resolve against the plugin directory; plain flags pass through.
    plugin_dir = os.path.dirname(manifest_path)
    argv = [a if os.path.isabs(a) or (os.sep not in a and not a.endswith(".py"))
            else os.path.normpath(os.path.join(plugin_dir, a))
            for a in argv]
    if os.sep in argv[0] and not os.path.exists(argv[0]):
        hint = manifest.get("build")
        raise FileNotFoundError(
            f"plugin {name!r} executable missing: {argv[0]}"
            + (f" — build it with: {hint}" if hint else ""))
    return argv, manifest
