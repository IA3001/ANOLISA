#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check that `component.toml` and the skill tree describe the same skills.

The packaging contract in `component.toml` decides which skills an adapter
publishes: a skill whose directory ships but which is missing from an
adapter's list is installed and discoverable, yet never handed to that
framework, and nothing reports it. The reverse — a listed name with no
directory — produces a bundle entry pointing at nothing.

These tests pin both directions against the real file, so a skill rename
or a hand-edited list fails here instead of in a released package.
"""

import re
from pathlib import Path
import unittest

COMPONENT_DIR = Path(__file__).resolve().parents[1]
COMPONENT_TOML = COMPONENT_DIR / "component.toml"

ENTRY = re.compile(
    r'\[\[adapters\.(?P<framework>[A-Za-z0-9_-]+)\.skills\]\]\s*\n'
    r'name = "(?P<name>[^"]+)"\s*\n'
    r'source = "(?P<source>[^"]+)"'
)


def adapter_entries() -> dict[str, list[tuple[str, str]]]:
    """Return {framework: [(name, source), ...]} from the packaging contract."""
    text = COMPONENT_TOML.read_text(encoding="utf-8")
    entries: dict[str, list[tuple[str, str]]] = {}
    for match in ENTRY.finditer(text):
        entries.setdefault(match.group("framework"), []).append(
            (match.group("name"), match.group("source")))
    return entries


def skill_dirs() -> set[str]:
    """Return the names of directories holding a SKILL.md under this component."""
    return {
        path.parent.name
        for path in COMPONENT_DIR.glob("*/*/SKILL.md")
        if path.parent.parent.parent == COMPONENT_DIR
    }


class ComponentAdapterParityTest(unittest.TestCase):
    def setUp(self) -> None:
        self.entries = adapter_entries()
        self.dirs = skill_dirs()

    def test_component_toml_declares_adapters(self) -> None:
        # Without this, the assertions below would pass on an empty parse and
        # the whole file could stop describing anything.
        self.assertTrue(self.entries, f"no adapter entries parsed from {COMPONENT_TOML}")
        self.assertIn("openclaw", self.entries)

    def test_every_entry_names_a_skill_directory(self) -> None:
        for framework, entries in self.entries.items():
            for name, _ in entries:
                with self.subTest(framework=framework, name=name):
                    self.assertIn(
                        name, self.dirs,
                        f"{framework} lists {name!r} but no {name}/SKILL.md exists")

    def test_every_entry_source_matches_its_name(self) -> None:
        for framework, entries in self.entries.items():
            for name, source in entries:
                with self.subTest(framework=framework, name=name):
                    self.assertEqual(
                        source, "{datadir}/skills/" + name + "/",
                        f"{framework} entry {name!r} points somewhere else")

    def test_every_skill_directory_is_published_by_every_adapter(self) -> None:
        # A shipped skill that is missing from an adapter is installed and
        # discoverable but never handed to that framework.
        for framework, entries in self.entries.items():
            listed = {name for name, _ in entries}
            with self.subTest(framework=framework):
                self.assertEqual(
                    sorted(self.dirs - listed), [],
                    f"{framework} does not publish these skill directories")

    def test_adapters_publish_the_same_skills(self) -> None:
        published = {
            framework: {name for name, _ in entries}
            for framework, entries in self.entries.items()
        }
        reference = published.get("openclaw", set())
        for framework, names in published.items():
            with self.subTest(framework=framework):
                self.assertEqual(sorted(names ^ reference), [],
                                 f"{framework} publishes a different set")


if __name__ == "__main__":
    unittest.main()
