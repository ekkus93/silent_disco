"""Regression tests for the GitHub-hosted workflow runner policy guard."""

import unittest

from check_hosted_runner_policy import check_workflow


class HostedRunnerPolicyTests(unittest.TestCase):
    def test_accepts_hosted_ubuntu_versions(self):
        for runner in ("ubuntu-24.04", "ubuntu-22.04", "ubuntu-latest"):
            with self.subTest(runner=runner):
                self.assertEqual(check_workflow("ci.yml", f"    runs-on: {runner}\n"), [])

    def test_rejects_self_hosted_and_dynamic_runners(self):
        for runner in ("[self-hosted, Linux, X64]", "self-hosted", "${{ matrix.runner }}"):
            with self.subTest(runner=runner):
                self.assertTrue(check_workflow("ci.yml", f"    runs-on: {runner}\n"))

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

    def test_hardware_stage_cannot_add_a_second_job(self):
        stage = (
            "jobs:\n  stage:\n    if: ${{ false }}\n"
            "    runs-on: [self-hosted, Linux, X64, silent-disco]\n"
            "  other:\n    runs-on: ubuntu-24.04\n"
        )
        self.assertTrue(check_workflow("block46-self-hosted-stage.yml", stage))


if __name__ == "__main__":
    unittest.main()
