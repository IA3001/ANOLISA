#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Regression tests for the string values the editors add to a workbook.

`add_shared_string` handed the caller's text straight to ElementTree. Text
carrying a control character then produced a sharedStrings.xml no parser
accepts, while the command still exited 0.
"""

import importlib.util
import os
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

sys.dont_write_bytecode = True

SCRIPTS_DIR = os.path.join(
    str(Path(__file__).resolve().parents[1]), "others", "xlsx", "scripts"
)
sys.path.insert(0, SCRIPTS_DIR)

NS_SST = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
SHARED_STRINGS = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="{NS_SST}" count="1" uniqueCount="1"><si><t>existing</t></si></sst>
"""


def load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(SCRIPTS_DIR, f"{name}.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AddSharedStringTest(unittest.TestCase):
    def assert_stored(self, editor, text, expected):
        module = load(editor)
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "xl", "sharedStrings.xml")
            os.makedirs(os.path.dirname(path))
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(SHARED_STRINGS)
            index = module.add_shared_string(directory, text)
            self.assertEqual(index, 1)
            payload = [t.text or "" for t in ET.parse(path).getroot().iter(f"{{{NS_SST}}}t")]
            self.assertEqual(payload[-1], expected)

    def test_insert_row_escapes_an_unstorable_character(self):
        self.assert_stored("xlsx_insert_row", "before\x0bafter", "before_x000B_after")

    def test_add_column_escapes_an_unstorable_character(self):
        self.assert_stored("xlsx_add_column", "before\x0bafter", "before_x000B_after")

    def test_ordinary_text_is_stored_verbatim(self):
        self.assert_stored("xlsx_insert_row", "a & b < c", "a & b < c")


if __name__ == "__main__":
    unittest.main()
