"""Regression tests for the GitHub-hosted workflow runner policy guard."""

import unittest

from check_hosted_runner_policy import check_workflow


class HostedRunnerPolicyTests(unittest.TestCase):
    def test_accepts_hosted_ubuntu_versions(self):
        for runner in ("ubuntu-24.04", "ubuntu-22.04", "ubuntu-latest"):
            with self.subTest(runner=runner):
                workflow = f"on:\n  push:\njobs:\n  quality:\n    runs-on: {runner}\n"
                self.assertEqual(check_workflow("ci.yml", workflow), [])

    def test_rejects_self_hosted_and_dynamic_runners(self):
        for runner in ("[self-hosted, Linux, X64]", "self-hosted", "${{ matrix.runner }}"):
            with self.subTest(runner=runner):
                workflow = f"on:\n  push:\njobs:\n  quality:\n    runs-on: {runner}\n"
                self.assertTrue(check_workflow("ci.yml", workflow))

    def test_hardware_stage_requires_job_level_false(self):
        stage = (
            "on:\n  workflow_dispatch:\njobs:\n  stage:\n"
            "    if: ${{ false }}\n"
            "    runs-on: [self-hosted, Linux, X64, silent-disco]\n"
        )
        self.assertEqual(check_workflow("block46-self-hosted-stage.yml", stage), [])
        self.assertTrue(
            check_workflow(
                "block46-self-hosted-stage.yml",
                stage.replace("if: ${{ false }}", "if: ${{ true }}"),
            )
        )

    def test_hardware_stage_cannot_gain_push_trigger(self):
        stage = (
            "on:\n  workflow_dispatch:\n  push:\njobs:\n  stage:\n"
            "    if: ${{ false }}\n"
            "    runs-on: [self-hosted, Linux, X64, silent-disco]\n"
        )
        self.assertTrue(check_workflow("block46-self-hosted-stage.yml", stage))

    def test_disabled_device_probe_is_manual_only(self):
        probe = (
            "on:\n  workflow_dispatch:\njobs:\n  probe:\n"
            "    if: ${{ false }}\n    runs-on: self-hosted\n"
        )
        self.assertEqual(check_workflow("self-hosted-android-device-probe.yml", probe), [])
        self.assertTrue(
            check_workflow(
                "self-hosted-android-device-probe.yml",
                probe.replace("if: ${{ false }}", "if: ${{ true }}"),
            )
        )
        self.assertTrue(
            check_workflow(
                "self-hosted-android-device-probe.yml",
                probe.replace("  workflow_dispatch:\n", "  workflow_dispatch:\n  push:\n"),
            )
        )

    def test_hardware_stage_cannot_add_a_second_job(self):
        stage = (
            "jobs:\n  stage:\n    if: ${{ false }}\n"
            "    runs-on: [self-hosted, Linux, X64, silent-disco]\n"
            "  other:\n    runs-on: ubuntu-24.04\n"
        )
        self.assertTrue(check_workflow("block46-self-hosted-stage.yml", stage))

    def test_hardware_stage_rejects_inline_and_additional_events(self):
        base = (
            "on:\n  workflow_dispatch:\njobs:\n  stage:\n"
            "    if: ${{ false }}\n"
            "    runs-on: [self-hosted, Linux, X64, silent-disco]\n"
        )
        self.assertEqual(check_workflow("block46-self-hosted-stage.yml", base), [])
        for trigger in (
            "on: [workflow_dispatch, push]\n",
            "on: workflow_dispatch\n",
            "on: {workflow_dispatch: {}, push: {}}\n",
            "on:\n  workflow_dispatch:\n  pull_request_target:\n",
            "on:\n  workflow_dispatch:\n  repository_dispatch:\n",
            "on:\n  workflow_dispatch:\n  workflow_call:\n",
            "on:\n  workflow_dispatch:\n  schedule:\n",
            "on:\n  workflow_dispatch:\n  merge_group:\n",
        ):
            with self.subTest(trigger=trigger):
                changed = base.replace("on:\n  workflow_dispatch:\n", trigger)
                self.assertTrue(check_workflow("block46-self-hosted-stage.yml", changed))

    def test_manual_dispatch_inputs_are_allowed_but_no_missing_event(self):
        stage = (
            "on:\n  # Keep the stage opt-in.\n"
            "  workflow_dispatch:\n"
            "    inputs:\n      reason:\n        required: true\n"
            "jobs:\n  stage:\n    if: ${{ false }}\n"
            "    runs-on: self-hosted\n"
        )
        self.assertEqual(check_workflow("block46-self-hosted-stage.yml", stage), [])
        self.assertTrue(check_workflow("block46-self-hosted-stage.yml", stage.replace(
            "on:\n  # Keep the stage opt-in.\n  workflow_dispatch:\n", ""
        )))


    def test_rejects_reusable_job_bypass_and_missing_runner(self):
        reusable = """on:
  push:
jobs:
  delegate:
    uses: example/repo/.github/workflows/unsafe.yml@main
"""
        self.assertTrue(check_workflow("ci.yml", reusable))
        self.assertTrue(check_workflow("ci.yml", """on:
  push:
jobs:
  empty:
    steps:
      - run: echo no-runner
"""))

    def test_rejects_hardware_second_job_without_runner(self):
        stage = """on:
  workflow_dispatch:
jobs:
  stage:
    if: \${{ false }}
    runs-on: self-hosted
  delegate:
    if: \${{ false }}
    uses: example/repo/.github/workflows/unsafe.yml@main
"""
        self.assertTrue(check_workflow("block46-self-hosted-stage.yml", stage))

    def test_accepts_two_literal_hosted_jobs_and_rejects_duplicate_runners(self):
        hosted = """on:
  push:
jobs:
  quality:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v5
  package:
    runs-on: ubuntu-22.04
"""
        self.assertEqual(check_workflow("ci.yml", hosted), [])
        self.assertTrue(check_workflow("ci.yml", hosted.replace(
            "    runs-on: ubuntu-22.04",
            "    runs-on: ubuntu-22.04\n    runs-on: self-hosted",
        )))
        self.assertTrue(check_workflow("ci.yml", "jobs: {unsafe: {runs-on: self-hosted}}\n"))


if __name__ == "__main__":
    unittest.main()
