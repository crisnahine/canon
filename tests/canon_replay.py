"""Shared plumbing for the harnesses that drive a real repository through canon.

Every one of them needs the same three things, and getting any of them wrong
makes the harness report success while measuring nothing.

  index    a snapshot has to exist before `inject` has any rule to apply. With
           none, `inject` answers `{}` and every case comes back clean.
  isolate  the snapshot goes in a data directory this run owns, so a harness
           never reads whatever the developer's own session left behind and
           never writes into it either.
  count    a case that produced no context block is not evidence of anything.
           A harness that only counts refusals cannot tell "no rule fired" from
           "no rule exists", which is exactly the state a missing snapshot
           leaves it in.
"""
import atexit
import json
import os
import shutil
import subprocess
import tempfile

_ROOT = None


def _workspace():
    """One temporary directory per run, removed when the process exits."""
    global _ROOT
    if _ROOT is None:
        _ROOT = tempfile.mkdtemp(prefix="canon-harness-")
        atexit.register(shutil.rmtree, _ROOT, True)
    return _ROOT


def index(binary, repo, name):
    """Build `repo`'s snapshot in a fresh data directory and return it.

    Raises rather than returning a directory with no snapshot in it: a harness
    that replays against an empty snapshot passes unconditionally.
    """
    data = os.path.join(_workspace(), name)
    os.makedirs(data, exist_ok=True)
    r = subprocess.run([binary, "index", "--rebuild"], cwd=repo, capture_output=True,
                       text=True, env={**os.environ, "CANON_DATA_DIR": data})
    if r.returncode != 0:
        raise SystemExit(f"index failed for {name}: exit {r.returncode}\n{r.stderr[:400]}")
    snapshots = os.path.join(data, "snapshots")
    if not os.path.isdir(snapshots) or not os.listdir(snapshots):
        raise SystemExit(f"index wrote no snapshot for {name}; the replay would measure nothing")
    return data


def inject(binary, data, cwd, rel, content):
    """Run one write through the hook.

    Returns `(verdict, why, blocked)`. `verdict` is None when canon allowed the
    write, `"DENY"` when it refused, and `"ERROR"` when the hook broke its
    fail-open contract. `blocked` says whether canon had anything to state
    about the file at all.
    """
    payload = json.dumps({"session_id": "replay", "cwd": cwd, "tool_name": "Write",
                          "tool_input": {"file_path": os.path.join(cwd, rel),
                                         "content": content}})
    r = subprocess.run([binary, "inject"], input=payload, capture_output=True, text=True,
                       cwd=cwd, env={**os.environ, "CANON_DATA_DIR": data})
    if r.returncode != 0 or r.stderr.strip():
        return ("ERROR", (r.stderr or f"exit {r.returncode}")[:160], False)
    try:
        out = json.loads(r.stdout or "{}")
    except ValueError:
        return ("ERROR", f"stdout is not JSON: {r.stdout[:160]}", False)
    hook = out.get("hookSpecificOutput", {})
    blocked = bool(hook.get("additionalContext"))
    if hook.get("permissionDecision") == "deny":
        why = hook.get("permissionDecisionReason", "").split("\n\n")
        return ("DENY", why[1].strip().replace("\n", " / ") if len(why) > 1 else "", blocked)
    return (None, "", blocked)


def repos(root):
    """Every checkout under `root/realrepos`, sorted."""
    base = os.path.join(root, "realrepos")
    return [(n, os.path.join(base, n)) for n in sorted(os.listdir(base))
            if os.path.isdir(os.path.join(base, n))]


def tracked(cwd):
    out = subprocess.run(["git", "ls-files"], cwd=cwd, capture_output=True, text=True).stdout
    return [f for f in out.split("\n") if f]
