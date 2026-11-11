#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Tests for style_audit.py worksheet part resolution and partial-audit reporting.

`workbook.xml.rels` Targets may be relative to `xl/`, carry `.`/`..` segments,
or be package-absolute. The loader resolved the first and third but not the
dot-segment form, and it dropped any sheet whose part it could not open
without saying so — so a workbook where one sheet failed to resolve was
audited over the remaining sheets and reported PASS.
"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

sys.dont_write_bytecode = True

SCRIPTS = Path(__file__).resolve().parents[1] / "others" / "xlsx" / "scripts"
sys.path.insert(0, str(SCRIPTS))
import style_audit as sa  # noqa: E402

SCRIPT = SCRIPTS / "style_audit.py"

CONTENT_TYPES = (
    '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/'
    'package/2006/content-types"><Default Extension="rels" ContentType='
    '"application/vnd.openxmlformats-package.relationships+xml"/><Default '
    'Extension="xml" ContentType="application/xml"/></Types>'
)
ROOT_RELS = (
    '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.'
    'org/package/2006/relationships"><Relationship Id="rId1" Type="http://'
    'schemas.openxmlformats.org/officeDocument/2006/relationships/office'
    'Document" Target="xl/workbook.xml"/></Relationships>'
)
# A complete-enough style sheet: the auditor requires the two mandatory fills
# and a font for every fontId a cell references.
STYLES = (
    '<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/'
    'spreadsheetml/2006/main"><numFmts count="0"/><fonts count="1">'
    '<font><sz val="11"/><name val="Calibri"/></font></fonts>'
    '<fills count="2"><fill><patternFill patternType="none"/></fill>'
    '<fill><patternFill patternType="gray125"/></fill></fills>'
    '<borders count="1"><border/></borders><cellStyleXfs count="1">'
    '<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
    '<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" '
    'xfId="0"/></cellXfs></styleSheet>'
)
WORKSHEET = (
    '<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/'
    'spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="0">'
    '<v>1</v></c></row></sheetData></worksheet>'
)


def workbook(sheet_names):
    sheets = "".join(
        f'<sheet name="{name}" sheetId="{index}" r:id="rId{index}"/>'
        for index, name in enumerate(sheet_names, start=1)
    )
    return (
        '<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.'
        'org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.'
        'org/officeDocument/2006/relationships"><sheets>' + sheets +
        '</sheets></workbook>'
    )


def rels(targets):
    entries = "".join(
        '<Relationship Id="rId%d" Type="http://schemas.openxmlformats.org/'
        'officeDocument/2006/relationships/worksheet" Target="%s"/>'
        % (index, target)
        for index, target in enumerate(targets, start=1)
    )
    return (
        '<?xml version="1.0"?><Relationships xmlns="http://schemas.'
        'openxmlformats.org/package/2006/relationships">' + entries +
        '</Relationships>'
    )


def build(path, names, targets, present):
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES)
        z.writestr("_rels/.rels", ROOT_RELS)
        z.writestr("xl/styles.xml", STYLES)
        z.writestr("xl/workbook.xml", workbook(names))
        z.writestr("xl/_rels/workbook.xml.rels", rels(targets))
        for part in present:
            z.writestr(part, WORKSHEET)
    return path


class TestResolveSheetTarget(unittest.TestCase):
    def test_relative(self):
        self.assertEqual(sa._resolve_sheet_target("worksheets/sheet1.xml"),
                         "xl/worksheets/sheet1.xml")

    def test_package_absolute(self):
        self.assertEqual(sa._resolve_sheet_target("/xl/worksheets/sheet1.xml"),
                         "xl/worksheets/sheet1.xml")

    def test_dot_segment(self):
        self.assertEqual(sa._resolve_sheet_target("./worksheets/sheet1.xml"),
                         "xl/worksheets/sheet1.xml")

    def test_parent_segment(self):
        self.assertEqual(
            sa._resolve_sheet_target("./worksheets/../worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml")


class TestLoadAndReport(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.path = os.path.join(self._tmp.name, "book.xlsx")

    def test_dot_segment_target_is_loaded(self):
        """The second sheet used to be dropped, leaving a one-sheet audit."""
        build(self.path,
              ["Plain", "Dotted"],
              ["worksheets/sheet1.xml", "./worksheets/sheet2.xml"],
              ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"])
        styles_xml, sheet_xmls, unreadable = sa._load_from_xlsx(self.path)

        self.assertEqual([name for name, _ in sheet_xmls], ["Plain", "Dotted"])
        self.assertEqual(unreadable, [])

    def test_missing_part_is_reported_not_dropped(self):
        build(self.path,
              ["Plain", "Lost"],
              ["worksheets/sheet1.xml", "worksheets/sheet2.xml"],
              ["xl/worksheets/sheet1.xml"])
        styles_xml, sheet_xmls, unreadable = sa._load_from_xlsx(self.path)

        self.assertEqual([name for name, _ in sheet_xmls], ["Plain"])
        self.assertEqual(len(unreadable), 1)
        self.assertIn("Lost", unreadable[0])
        self.assertIn("sheet2.xml", unreadable[0])

    def test_cli_fails_on_a_partial_audit(self):
        """A skipped sheet must not read as a clean audit."""
        build(self.path,
              ["Plain", "Lost"],
              ["worksheets/sheet1.xml", "worksheets/sheet2.xml"],
              ["xl/worksheets/sheet1.xml"])
        result = subprocess.run([sys.executable, str(SCRIPT), self.path],
                                capture_output=True, text=True)

        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("could not be read", result.stdout)
        self.assertNotIn("PASS", result.stdout)

    def test_cli_passes_when_every_sheet_loads(self):
        build(self.path,
              ["Plain", "Dotted"],
              ["worksheets/sheet1.xml", "./worksheets/sheet2.xml"],
              ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"])
        result = subprocess.run([sys.executable, str(SCRIPT), self.path],
                                capture_output=True, text=True)

        self.assertIn("PASS", result.stdout, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
