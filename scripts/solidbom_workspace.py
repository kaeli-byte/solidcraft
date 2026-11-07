#!/usr/bin/env python3
"""Validate SolidBOM handoffs and restore hash-pinned local reference files."""

import argparse
import fnmatch
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs/solidbom"
STATES = {
    "planned", "ready", "in_progress", "in_review", "ready_to_integrate",
    "integrated", "verified", "blocked",
}
INTEGRATED = {"integrated", "verified"}
ACTIVE = {"ready", "in_progress", "in_review", "ready_to_integrate", *INTEGRATED}
HEADINGS = (
    "Objective", "Requirements", "Dependencies", "Scope", "Interfaces",
    "Ownership", "Acceptance", "Assignment gate", "Handoff",
)


def git(*args, cwd=ROOT):
    return subprocess.run(
        ["git", *args], cwd=cwd, text=True, capture_output=True, check=False,
    )


def load_json(path):
    return json.loads(path.read_text())


def safe_relative(value):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or ".." in path.parts or "\\" in value:
        raise ValueError(f"unsafe manifest path: {value!r}")
    return Path(*path.parts)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def manifest():
    data = load_json(DOCS / "reference-manifest.json")
    if data.get("schema_version") != 1:
        raise ValueError("unsupported reference manifest version")
    if data.get("destination") != "docs/solidbom/requirements":
        raise ValueError("unexpected reference destination")
    seen = set()
    for item in data["files"]:
        safe_relative(item["path"])
        if item["path"] in seen:
            raise ValueError(f"duplicate reference: {item['path']}")
        seen.add(item["path"])
        expected_storage = "git" if Path(item["path"]).suffix == ".md" else "local"
        if item.get("storage") != expected_storage:
            raise ValueError(f"invalid storage class: {item['path']}")
        if not isinstance(item.get("bytes"), int) or item["bytes"] < 0:
            raise ValueError(f"invalid byte size: {item['path']}")
        if not re.fullmatch(r"[0-9a-f]{64}", item.get("sha256", "")):
            raise ValueError(f"invalid source hash: {item['path']}")
    return data


def matches(path, item):
    return path.is_file() and path.stat().st_size == item["bytes"] and digest(path) == item["sha256"]


def records():
    index = load_json(DOCS / "package-index.json")
    if index.get("schema_version") != 1:
        raise ValueError("unsupported package index version")
    packages = index["packages"]
    ids = [p["id"] for p in packages]
    if len(set(ids)) != len(ids):
        raise ValueError("duplicate package IDs")
    result = {}
    for package in packages:
        pid = package["id"]
        if not re.fullmatch(r"SB-(?:[0-9]{2}|X[0-9]{2})", pid):
            raise ValueError(f"invalid package ID: {pid}")
        path = DOCS / "execution" / f"{pid}.md"
        block = re.search(r"```json\s*\n(.*?)\n```", path.read_text(), re.S)
        if not block:
            raise ValueError(f"{path}: missing JSON execution record")
        result[pid] = json.loads(block.group(1))
    return packages, result


def actual_commit(value):
    return isinstance(value, str) and bool(re.fullmatch(r"[0-9a-f]{40}", value)) and git(
        "cat-file", "-e", f"{value}^{{commit}}",
    ).returncode == 0


def covers(ancestor, descendant):
    return actual_commit(ancestor) and actual_commit(descendant) and git(
        "merge-base", "--is-ancestor", ancestor, descendant,
    ).returncode == 0


def passed_checks(value):
    return isinstance(value, list) and bool(value) and all(
        isinstance(item, dict)
        and all(item.get(key) for key in ("command", "evidence"))
        and item.get("result") == "passed"
        and actual_commit(item.get("commit"))
        for item in value
    )


def check_packages():
    packages, progress = records()
    problems = []
    graph = {p["id"]: p["dependencies"] for p in packages}
    visiting, visited = set(), set()

    def visit(pid):
        if pid in visiting:
            raise ValueError(f"dependency cycle at {pid}")
        if pid in visited:
            return
        visiting.add(pid)
        for dep in graph[pid]:
            if dep not in graph:
                raise ValueError(f"{pid}: unknown prerequisite {dep}")
            visit(dep)
        visiting.remove(pid)
        visited.add(pid)

    for pid in graph:
        visit(pid)
    ledger = (DOCS / "EXECUTION.md").read_text()
    for package in packages:
        pid = package["id"]
        record = progress[pid]
        if not isinstance(record, dict):
            problems.append(f"{pid}: execution record is not a JSON object")
            continue
        brief = (DOCS / "packages" / f"{pid}.md").read_text()
        for heading in HEADINGS:
            if not re.search(rf"^## {re.escape(heading)}\s*$", brief, re.M):
                problems.append(f"{pid}: missing brief heading {heading}")
        dependency_line = re.search(r"^Depends on:\s*(.*)$", brief, re.M)
        declared = re.findall(r"SB-(?:[0-9]{2}|X[0-9]{2})", dependency_line.group(1)) if dependency_line else None
        if declared != package["dependencies"]:
            problems.append(f"{pid}: brief dependencies differ from package index")
        status = record.get("status")
        if record.get("package") != pid or status not in STATES:
            problems.append(f"{pid}: invalid execution identity/status")
        row = next((line for line in ledger.splitlines() if line.startswith(f"| [{pid}]")), "")
        if not row or f"| {status} |" not in row:
            problems.append(f"{pid}: coordinator ledger disagrees with execution record")
        if row:
            cells = [cell.strip() for cell in row.strip().strip("|").split("|")]
            if len(cells) > 2:
                ledger_dependencies = re.findall(r"SB-(?:[0-9]{2}|X[0-9]{2})", cells[2])
                if ledger_dependencies != package["dependencies"]:
                    problems.append(f"{pid}: coordinator ledger dependencies differ from package index")
        if status == "blocked":
            blocker = record.get("blocker") or {}
            if not all(blocker.get(key) for key in ("cause", "owner", "resume_action")):
                problems.append(f"{pid}: blocked state needs cause, owner and resume_action")
            if record.get("previous_state") not in STATES - {"blocked"}:
                problems.append(f"{pid}: blocked state needs previous_state")
        if status in ACTIVE:
            if not record.get("brief_ready"):
                problems.append(f"{pid}: active package has no completed brief")
            for dep in package["dependencies"]:
                dependency = progress[dep]
                commit = (record.get("dependency_commits") or {}).get(dep)
                if not isinstance(dependency, dict) or dependency.get("status") not in INTEGRATED or not actual_commit(commit):
                    problems.append(f"{pid}: prerequisite {dep} is not integrated/pinned")
                elif commit != dependency.get("integrated_commit"):
                    problems.append(f"{pid}: prerequisite {dep} differs from its integration record")
                elif git("merge-base", "--is-ancestor", commit, "HEAD").returncode:
                    problems.append(f"{pid}: prerequisite {dep} is absent from this checkout")
            if pid not in {"SB-00", "SB-01"} and not record.get("locked_contracts"):
                problems.append(f"{pid}: active package has no locked contracts")
            if pid == "SB-01":
                review = record.get("brief_review") or {}
                commit = review.get("commit")
                if not review.get("reviewer") or not actual_commit(commit):
                    problems.append(f"{pid}: foundation assignment needs an independently reviewed brief")
                elif git("cat-file", "-e", f"{commit}:docs/solidbom/packages/SB-01.md").returncode:
                    problems.append(f"{pid}: foundation brief is absent from reviewed commit")
                elif git("merge-base", "--is-ancestor", commit, "HEAD").returncode:
                    problems.append(f"{pid}: reviewed foundation brief is absent from this checkout")
            for contract in record.get("locked_contracts") or []:
                path = safe_relative(contract["path"])
                commit = contract.get("commit")
                if not contract.get("version") or not actual_commit(commit):
                    problems.append(f"{pid}: contract must pin a version and actual commit")
                elif git("cat-file", "-e", f"{commit}:{path.as_posix()}").returncode:
                    problems.append(f"{pid}: contract does not exist in pinned commit: {path}")
                elif git("merge-base", "--is-ancestor", commit, "HEAD").returncode:
                    problems.append(f"{pid}: contract commit is absent from this checkout")
        if status in ACTIVE - {"ready"}:
            if not all(record.get(k) for k in ("agent", "worktree", "branch")) or not actual_commit(record.get("starting_commit")):
                problems.append(f"{pid}: active work needs owner/worktree/branch/actual base")
        for commit in record.get("implementation_commits") or []:
            if not actual_commit(commit):
                problems.append(f"{pid}: implementation commit does not exist: {commit}")
        if status in {"ready_to_integrate", *INTEGRATED}:
            reviewed = record.get("reviewed_commit")
            if not record.get("implementation_commits") or not record.get("reviewer") or not actual_commit(reviewed):
                problems.append(f"{pid}: integration gate needs implementation and independent review commits")
            if record.get("reviewer") == record.get("agent"):
                problems.append(f"{pid}: package author cannot be its independent reviewer")
            checks = record.get("acceptance_checks")
            if not passed_checks(checks):
                problems.append(f"{pid}: integration gate needs structured, passed acceptance checks")
            else:
                for item in checks:
                    if not covers(item["commit"], reviewed):
                        problems.append(f"{pid}: acceptance check commit is not covered by the reviewed commit")
            for commit in record.get("implementation_commits") or []:
                if actual_commit(commit) and not covers(commit, reviewed):
                    problems.append(f"{pid}: reviewed commit does not cover implementation commit {commit}")
        if status in INTEGRATED:
            commit = record.get("integrated_commit")
            if not actual_commit(commit) or git("merge-base", "--is-ancestor", commit, "HEAD").returncode:
                problems.append(f"{pid}: integration commit is absent from this checkout")
            elif not covers(reviewed, commit):
                problems.append(f"{pid}: integration commit does not cover reviewed commit")
        if status == "verified":
            checks = record.get("post_integration_checks")
            if not passed_checks(checks):
                problems.append(f"{pid}: verified state needs structured, passed post-integration evidence")
            else:
                for item in checks:
                    if not covers(item["commit"], "HEAD"):
                        problems.append(f"{pid}: post-integration check commit is absent from this checkout")
    return problems


def check(require_references):
    data = manifest()
    problems = check_packages()
    missing_local = set()
    for item in data["files"]:
        path = ROOT / data["destination"] / safe_relative(item["path"])
        if not path.exists() and item["storage"] == "local" and not require_references:
            missing_local.add(path.resolve())
        elif not matches(path, item):
            problems.append(f"missing or changed source file: {item['path']}")
        ignored = git("check-ignore", "--no-index", "-q", str(path)).returncode == 0
        if ignored != (item["storage"] == "local"):
            problems.append(f"incorrect Git ignore classification: {item['path']}")
        if item["storage"] == "local" and git("ls-files", "--error-unmatch", str(path)).returncode == 0:
            problems.append(f"private reference is tracked: {item['path']}")
    include = ROOT / ".worktreeinclude"
    patterns = [
        line.strip()
        for line in (include.read_text().splitlines() if include.is_file() else [])
        if line.strip() and not line.strip().startswith("#")
    ]
    for item in data["files"]:
        if item["storage"] != "local":
            continue
        relative = "/" + (Path(data["destination"]) / safe_relative(item["path"])).as_posix()
        if not any(fnmatch.fnmatchcase(relative, pattern) for pattern in patterns):
            problems.append(f"local reference is not copied by .worktreeinclude: {item['path']}")
    for path in DOCS.rglob("*.md"):
        for link in re.findall(r"\]\(([^)]+)\)", path.read_text()):
            target = unquote(link.strip("<>").split("#", 1)[0])
            if not target or re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*:", target):
                continue
            resolved = (path.parent / target).resolve()
            if not resolved.exists() and resolved not in missing_local:
                problems.append(f"{path.relative_to(ROOT)}: missing link target {target}")
    if problems:
        raise ValueError("\n".join(problems))
    package_count = len(load_json(DOCS / "package-index.json")["packages"])
    print(f"SolidBOM check passed: {len(data['files'])} source files, {package_count} package records; {len(missing_local)} optional local files absent.")


def hydrate(source, workspace):
    data = manifest()
    source, workspace = source.resolve(), workspace.resolve()
    if git("rev-parse", "--show-toplevel", cwd=workspace).stdout.strip() != str(workspace):
        raise ValueError("--workspace must be a Git checkout root")
    copies = []
    # Validate the whole snapshot and every conflict before creating any destination.
    for item in data["files"]:
        relative = safe_relative(item["path"])
        original = source / relative
        if not matches(original, item):
            raise ValueError(f"source does not match manifest: {item['path']}")
        if item["storage"] != "local":
            continue
        destination = workspace / data["destination"] / relative
        if not destination.resolve().is_relative_to(workspace):
            raise ValueError(f"reference destination escapes workspace: {item['path']}")
        parent = destination.parent
        while parent != workspace and parent.is_relative_to(workspace):
            if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
                raise ValueError(f"reference parent is not a directory: {parent}")
            parent = parent.parent
        if destination.exists() and not matches(destination, item):
            raise ValueError(f"refusing to overwrite differing reference: {destination}")
        if git("ls-files", "--error-unmatch", str(destination), cwd=workspace).returncode == 0:
            raise ValueError(f"refusing to hydrate tracked private reference: {destination}")
        if git("check-ignore", "--no-index", "-q", str(destination), cwd=workspace).returncode:
            raise ValueError(f"reference must be Git-ignored before copying: {destination}")
        if not destination.exists():
            copies.append((original, destination))
    for original, destination in copies:
        destination.parent.mkdir(parents=True, exist_ok=True)
        try:
            with original.open("rb") as incoming, destination.open("xb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)
        except FileExistsError as error:
            raise ValueError(f"refusing to overwrite differing reference: {destination}") from error
        except BaseException:
            destination.unlink(missing_ok=True)
            raise
    print(f"Hydrated {len(copies)} references into {workspace}; existing matching files preserved.")


def status():
    packages, progress = records()
    for package in packages:
        record = progress[package["id"]]
        waiting = [
            dep for dep in package["dependencies"]
            if not isinstance(progress.get(dep), dict) or progress[dep].get("status") not in INTEGRATED
        ]
        note = "waiting for " + ", ".join(waiting) if waiting else "prerequisites satisfied; coordinator must verify contracts, ancestry and assignment gates"
        print(f"{package['id']:6} {record['status']:19} {note}")


def source_ci():
    """Run the unchanged source gate without private, untracked owner assets."""
    data = manifest()
    present = []
    for item in data["files"]:
        if item["storage"] != "local":
            continue
        path = ROOT / data["destination"] / safe_relative(item["path"])
        if path.is_symlink() or not path.resolve().is_relative_to(ROOT):
            raise ValueError(f"refusing to detach a symlinked reference: {path}")
        if path.exists():
            if not matches(path, item):
                raise ValueError(f"changed local reference; preserve it before running source-ci: {path}")
            relative = path.relative_to(ROOT)
            if git("ls-files", "--error-unmatch", str(relative)).returncode == 0:
                raise ValueError(f"refusing to detach tracked file: {path}")
            if git("check-ignore", "--no-index", "-q", str(relative)).returncode:
                raise ValueError(f"refusing to detach nonignored file: {path}")
            present.append((path, relative))
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    stash = Path(tempfile.mkdtemp(prefix="solidbom-reference-stash-", dir=target))
    (stash / "recovery.json").write_text(json.dumps([str(rel) for _, rel in present], indent=2) + "\n")
    moved = []
    conflicts = []
    try:
        for path, relative in present:
            backup = stash / relative
            backup.parent.mkdir(parents=True, exist_ok=True)
            path.rename(backup)
            moved.append((path, backup))
        print(f"Temporarily detached {len(moved)} ignored owner references; recovery map: {stash / 'recovery.json'}", flush=True)
        return subprocess.run(["cargo", "xtask", "ci"], cwd=ROOT, check=False).returncode
    finally:
        for path, backup in reversed(moved):
            if path.exists() or path.is_symlink():
                conflicts.append(str(path))
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                backup.rename(path)
        if conflicts:
            # Retain backups and recovery map rather than overwrite a concurrent edit.
            raise ValueError(f"reference restoration conflicts: {conflicts}; backups retained at {stash}")
        shutil.rmtree(stash)
        print(f"Restored {len(moved)} local references.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    checking = commands.add_parser("check", help="validate documents, source hashes and tracking gates")
    checking.add_argument("--require-references", action="store_true")
    restoring = commands.add_parser("hydrate", help="copy only hash-pinned ignored references")
    restoring.add_argument("--source", type=Path, required=True)
    restoring.add_argument("--workspace", type=Path, default=ROOT)
    commands.add_parser("status", help="show package state and dependency eligibility")
    commands.add_parser("source-ci", help="temporarily detach ignored owner references, run cargo xtask ci, then restore them")
    args = parser.parse_args()
    try:
        if args.command == "check":
            check(args.require_references)
        elif args.command == "hydrate":
            hydrate(args.source, args.workspace)
        elif args.command == "source-ci":
            return source_ci()
        else:
            status()
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"SolidBOM: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
