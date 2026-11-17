"""Tests for the npm audit triage policy used by check-supply-chain.sh."""

import datetime
import unittest

from npm_audit_triage import triage

TODAY = datetime.date(2026, 11, 17)
GHSA = "GHSA-fc5q-39xc-vjpf"


def report(*advisory_ids):
    """An `npm audit --json` (v2) shaped report with one package per advisory,
    plus a dependent package that only names it, as npm reports chains."""
    vulnerabilities = {}
    for index, advisory_id in enumerate(advisory_ids):
        name = f"pkg{index}"
        vulnerabilities[name] = {
            "via": [
                {
                    "source": 1000 + index,
                    "name": name,
                    "title": f"problem {index}",
                    "url": f"https://github.com/advisories/{advisory_id}",
                    "severity": "high",
                }
            ]
        }
        vulnerabilities[f"dependent-of-{name}"] = {"via": [name]}
    return {"auditReportVersion": 2, "vulnerabilities": vulnerabilities}


def exception(advisory=GHSA, until="2027-02-17", reason="no fix; dev only"):
    return {"advisory": advisory, "until": until, "reason": reason}


class NpmAuditTriageTests(unittest.TestCase):
    def test_clean_report_passes(self):
        self.assertEqual(triage(report(), [], TODAY), [])

    def test_unexcepted_advisory_fails(self):
        problems = triage(report(GHSA), [], TODAY)
        self.assertEqual(len(problems), 1)
        self.assertIn(GHSA, problems[0])
        self.assertIn("pkg0", problems[0])

    def test_excepted_advisory_passes(self):
        self.assertEqual(triage(report(GHSA), [exception()], TODAY), [])

    def test_exception_covers_only_its_own_advisory(self):
        problems = triage(report(GHSA, "GHSA-aaaa-bbbb-cccc"), [exception()], TODAY)
        self.assertEqual(len(problems), 1)
        self.assertIn("GHSA-aaaa-bbbb-cccc", problems[0])

    def test_expired_exception_fails(self):
        problems = triage(report(GHSA), [exception(until="2026-11-16")], TODAY)
        self.assertTrue(any("expired" in problem for problem in problems))
        self.assertTrue(any(GHSA in problem and "high" in problem for problem in problems))

    def test_exception_on_its_last_day_still_applies(self):
        self.assertEqual(triage(report(GHSA), [exception(until="2026-11-17")], TODAY), [])

    def test_unused_exception_fails(self):
        problems = triage(report(), [exception()], TODAY)
        self.assertEqual(problems, [f"exception for {GHSA} matches no finding; remove it"])

    def test_exception_without_reason_fails(self):
        problems = triage(report(GHSA), [exception(reason="  ")], TODAY)
        self.assertTrue(any("needs advisory, reason and until" in problem for problem in problems))

    def test_audit_that_did_not_run_fails(self):
        problems = triage({"error": {"code": "ENOTFOUND", "summary": "registry unreachable"}}, [], TODAY)
        self.assertEqual(len(problems), 1)
        self.assertIn("did not run", problems[0])

    def test_output_without_vulnerability_section_fails(self):
        self.assertTrue(triage({"auditReportVersion": 2}, [], TODAY))


if __name__ == "__main__":
    unittest.main()
