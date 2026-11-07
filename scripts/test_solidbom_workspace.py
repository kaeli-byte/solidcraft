"""Safety checks for local references and package assignment gates."""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("solidbom_workspace", Path(__file__).with_name("solidbom_workspace.py"))
WORKSPACE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WORKSPACE)


class WorkspaceChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout"
        self.source = Path(self.temporary.name) / "owner"
        self.docs = self.root / "docs/solidbom"
        self.docs.mkdir(parents=True)
        self.source.mkdir()
        self.root = self.root.resolve()
        self.source = self.source.resolve()
        self.docs = self.root / "docs/solidbom"
        self.files = []
        for name, content in [("hand/requirements.md", b"requirements"), ("hand/a.csv", b"first"), ("hand/b.pdf", b"second")]:
            path = self.source / name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(content)
            self.files.append({"path": name, "storage": "git" if name.endswith(".md") else "local", "bytes": len(content), "sha256": WORKSPACE.digest(path)})
        (self.docs / "reference-manifest.json").write_text(json.dumps({"schema_version": 1, "destination": "docs/solidbom/requirements", "files": self.files}))
        for name, value in [("ROOT", self.root), ("DOCS", self.docs), ("git", self.git)]:
            changing = patch.object(WORKSPACE, name, value)
            changing.start()
            self.addCleanup(changing.stop)
        self.tracked = False

    def git(self, *args, **kwargs):
        code = 1 if args[0] == "ls-files" and not self.tracked else 0
        return subprocess.CompletedProcess(args, code, str(self.root) + "\n" if args[0] == "rev-parse" else "", "")

    def destination(self, name):
        return self.docs / "requirements/hand" / name

    def test_hydration_is_repeatable_and_copies_only_local_files(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            WORKSPACE.hydrate(self.source, self.root)
            WORKSPACE.hydrate(self.source, self.root)
        self.assertIn("Hydrated 0 references", output.getvalue())
        self.assertEqual(self.destination("a.csv").read_bytes(), b"first")
        self.assertEqual(self.destination("b.pdf").read_bytes(), b"second")
        self.assertFalse(self.destination("requirements.md").exists())

    def test_destination_conflict_prevents_all_copies(self):
        self.destination("b.pdf").parent.mkdir(parents=True)
        self.destination("b.pdf").write_bytes(b"local edit")
        with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
            WORKSPACE.hydrate(self.source, self.root)
        self.assertFalse(self.destination("a.csv").exists())
        self.assertEqual(self.destination("b.pdf").read_bytes(), b"local edit")

    def test_changed_source_prevents_all_copies(self):
        (self.source / "hand/b.pdf").write_bytes(b"changed source")
        with self.assertRaisesRegex(ValueError, "source does not match"):
            WORKSPACE.hydrate(self.source, self.root)
        self.assertFalse(self.destination("a.csv").exists())

    def test_missing_source_prevents_all_copies(self):
        (self.source / "hand/b.pdf").unlink()
        with self.assertRaisesRegex(ValueError, "source does not match"):
            WORKSPACE.hydrate(self.source, self.root)
        self.assertFalse(self.destination("a.csv").exists())

    def test_destination_parent_that_is_not_a_directory_prevents_all_copies(self):
        parent = self.destination("a.csv").parent
        parent.parent.mkdir(parents=True)
        parent.write_bytes(b"not a directory")
        with self.assertRaisesRegex(ValueError, "parent is not a directory"):
            WORKSPACE.hydrate(self.source, self.root)
        self.assertEqual(parent.read_bytes(), b"not a directory")

    def test_tracked_private_reference_is_rejected(self):
        self.tracked = True
        with self.assertRaisesRegex(ValueError, "tracked"):
            WORKSPACE.hydrate(self.source, self.root)
        self.assertFalse(self.destination("a.csv").exists())

    def test_source_ci_restores_references_after_gate_failure(self):
        WORKSPACE.hydrate(self.source, self.root)

        def failed_gate(*args, **kwargs):
            self.assertFalse(self.destination("a.csv").exists())
            self.assertFalse(self.destination("b.pdf").exists())
            return subprocess.CompletedProcess(args, 7)

        with patch.object(WORKSPACE.subprocess, "run", failed_gate):
            self.assertEqual(WORKSPACE.source_ci(), 7)
        self.assertEqual(self.destination("a.csv").read_bytes(), b"first")
        self.assertEqual(self.destination("b.pdf").read_bytes(), b"second")
        self.assertEqual(list((self.root / "target").iterdir()), [])

    def package_fixture(self):
        commit = "a" * 40
        packages = [{"id": "SB-00", "dependencies": []}, {"id": "SB-01", "dependencies": ["SB-00"]}]
        records = {
            "SB-00": {"package": "SB-00", "status": "integrated", "brief_ready": True, "agent": "author", "reviewer": "reviewer", "worktree": "checkout", "branch": "bootstrap", "starting_commit": commit, "implementation_commits": [commit], "reviewed_commit": commit, "integrated_commit": commit, "review_evidence": "independent review log", "acceptance_checks": [{"command": "package check", "result": "passed", "commit": commit, "evidence": "package log"}]},
            "SB-01": {"package": "SB-01", "status": "ready", "brief_ready": True, "dependency_commits": {"SB-00": commit}, "locked_contracts": [], "brief_review": {"reviewer": "reviewer", "commit": commit}},
        }
        (self.docs / "packages").mkdir()
        for package in packages:
            brief = "Depends on: " + ", ".join(package["dependencies"]) + "\n"
            brief += "\n".join("## " + heading for heading in WORKSPACE.HEADINGS)
            (self.docs / "packages" / (package["id"] + ".md")).write_text(brief)
        (self.docs / "EXECUTION.md").write_text("| [SB-00] | integrated |\n| [SB-01] | ready |\n")
        return packages, records

    def test_contract_foundation_can_be_ready_before_first_product_contract(self):
        packages, records = self.package_fixture()
        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True):
            self.assertEqual(WORKSPACE.check_packages(), [])

    def test_verified_rejects_unstructured_or_failed_check_claims(self):
        packages, records = self.package_fixture()
        records["SB-00"]["status"] = "verified"
        records["SB-00"]["post_integration_checks"] = ["not actually run"]
        (self.docs / "EXECUTION.md").write_text("| [SB-00] | verified |\n| [SB-01] | ready |\n")
        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True):
            self.assertTrue(any("post-integration" in problem for problem in WORKSPACE.check_packages()))

    def test_integration_rejects_review_of_unrelated_commit(self):
        packages, records = self.package_fixture()
        unrelated = "b" * 40
        records["SB-00"]["reviewed_commit"] = unrelated

        def git_with_unrelated_review(*args, **kwargs):
            if args[:2] == ("merge-base", "--is-ancestor") and args[2] == "a" * 40 and args[3] == unrelated:
                return subprocess.CompletedProcess(args, 1, "", "")
            return self.git(*args, **kwargs)

        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True), patch.object(WORKSPACE, "git", git_with_unrelated_review):
            self.assertTrue(any("reviewed commit does not cover implementation commit" in problem for problem in WORKSPACE.check_packages()))

    def test_verified_rejects_a_structured_failed_check(self):
        packages, records = self.package_fixture()
        records["SB-00"]["status"] = "verified"
        records["SB-00"]["post_integration_checks"] = [
            {"command": "package check", "result": "failed", "commit": "a" * 40, "evidence": "package log"}
        ]
        (self.docs / "EXECUTION.md").write_text("| [SB-00] | verified |\n| [SB-01] | ready |\n")
        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True):
            self.assertTrue(any("post-integration" in problem for problem in WORKSPACE.check_packages()))

    def test_integration_rejects_an_acceptance_check_from_an_unreviewed_commit(self):
        packages, records = self.package_fixture()
        records["SB-00"]["reviewed_commit"] = "b" * 40
        records["SB-00"]["acceptance_checks"][0]["commit"] = "c" * 40

        def git_with_unrelated_commits(*args, **kwargs):
            if args[:2] == ("merge-base", "--is-ancestor") and args[2] != args[3]:
                return subprocess.CompletedProcess(args, 1, "", "")
            return self.git(*args, **kwargs)

        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True), patch.object(WORKSPACE, "git", git_with_unrelated_commits):
            self.assertTrue(any("acceptance check" in problem and "review" in problem for problem in WORKSPACE.check_packages()))

    def test_null_dependency_commits_is_reported_without_crashing(self):
        packages, records = self.package_fixture()
        records["SB-00"]["status"] = "in_progress"
        records["SB-01"]["dependency_commits"] = None
        (self.docs / "EXECUTION.md").write_text("| [SB-00] | in_progress |\n| [SB-01] | ready |\n")
        with patch.object(WORKSPACE, "records", return_value=(packages, records)), patch.object(WORKSPACE, "actual_commit", return_value=True):
            self.assertTrue(any("prerequisite SB-00" in problem for problem in WORKSPACE.check_packages()))


if __name__ == "__main__":
    unittest.main()
