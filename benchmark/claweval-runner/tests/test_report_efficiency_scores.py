# Copyright 2026 Alibaba Cloud
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Reports must show the converter's efficiency values, not the grader's zeros.

`grade()` cannot compute the efficiency dimensions, so `grading_result` carries
them at 0.0. Both report generators used to read only that event, so every
report claimed the trial was maximally token- and time-efficient.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

RUNNER = Path(__file__).resolve().parent.parent
SCRIPTS = RUNNER / "scripts"
CONVERTER = {
    "efficiency_turns": 0.8,
    "efficiency_tokens": 0.42,
    "efficiency_wall_time_s": 0.31,
}


def _load(name: str):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _tasks_dir(tmp_path: Path) -> str:
    task = tmp_path / "tasks" / "T001"
    task.mkdir(parents=True)
    (task / "task.yaml").write_text("task_id: T001\ntask_name: demo\n", encoding="utf-8")
    return str(tmp_path / "tasks")


def _trace(tmp_path: Path, *, trace_scores: dict | None) -> Path:
    grading = {
        "type": "grading_result",
        "passed": True,
        "task_score": 1.0,
        # What `grade()` reports for the dimensions it cannot compute.
        "scores": {**{key: 0.0 for key in CONVERTER}, "completion": 1.0},
        "judge_calls": [],
    }
    trace_end = {"type": "trace_end", "wall_time_s": 12.3, "total_turns": 5}
    if trace_scores is not None:
        trace_end["scores"] = trace_scores
    path = tmp_path / "traces" / "T001_trial0.jsonl"
    path.parent.mkdir(parents=True)
    path.write_text(
        "\n".join(json.dumps(event) for event in (grading, trace_end)) + "\n",
        encoding="utf-8",
    )
    return path


def _report_for_trial(tmp_path: Path, *, trace_scores: dict | None) -> dict:
    module = _load("generate_trial_reports")
    trace = _trace(tmp_path, trace_scores=trace_scores)
    _name, report = module.process_one_trace(
        str(trace), {"tasks_dir": _tasks_dir(tmp_path)}
    )
    return report


def test_generate_trial_reports_uses_the_converter_efficiency_values(tmp_path):
    report = _report_for_trial(tmp_path, trace_scores=CONVERTER)

    assert report["scores"]["efficiency_tokens"] == pytest.approx(0.42)
    assert report["scores"]["efficiency_turns"] == pytest.approx(0.8)
    assert report["scores"]["efficiency_wall_time_s"] == pytest.approx(0.31)
    # The grader's rubric dimensions stay the grader's.
    assert report["scores"]["completion"] == pytest.approx(1.0)


def test_generate_trial_reports_keeps_grading_values_without_converter_scores(tmp_path):
    report = _report_for_trial(tmp_path, trace_scores=None)

    assert report["scores"]["efficiency_tokens"] == 0.0
    assert report["scores"]["completion"] == pytest.approx(1.0)


def test_analyze_reports_use_the_converter_efficiency_values(tmp_path):
    module = _load("analyze")
    trace = _trace(tmp_path, trace_scores=CONVERTER)
    report_dir = tmp_path / "reports"

    processed = module.generate_reports(
        str(trace.parent), _tasks_dir(tmp_path), str(report_dir), "", "", ""
    )

    assert processed == 1
    written = [path for path in report_dir.glob("*.json")]
    assert written, "generate_reports wrote no report"
    report = json.loads(written[0].read_text(encoding="utf-8"))
    assert report["scores"]["efficiency_tokens"] == pytest.approx(0.42)
    assert report["scores"]["completion"] == pytest.approx(1.0)


def test_converter_values_that_are_not_numbers_are_ignored(tmp_path):
    report = _report_for_trial(
        tmp_path, trace_scores={**CONVERTER, "efficiency_tokens": "0.42"}
    )

    assert report["scores"]["efficiency_tokens"] == 0.0
    assert report["scores"]["efficiency_turns"] == pytest.approx(0.8)
