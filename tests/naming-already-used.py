"""Ask every blocking naming rule to refuse a filename the repository uses.

The replays cannot find this. Both build their candidates out of source files
in code directories, and the defect this exists for lived in a tree of JSON
fixtures they skip by name.

For every naming rule graded Blocking, this proposes every distinct filename
the repository already uses for the same extension somewhere else, and sorts
the refusals by where that filename actually lives:

  inside     under the very directory the rule speaks for. The rule reached
             total agreement without counting that file, so its own scope
             holds a counterexample and it may not refuse. This fails the run.
  declared   under some other directory canon states the same naming style
             for. Two readings of one tree disagree; reported, because whether
             it is wrong depends on whether the two directories are really one
             naming domain.
  elsewhere  a directory canon says nothing about, or says something else
             about. An ordinary style difference between two parts of a
             repository, and refusing is what a blocking naming rule is for.

Usage: naming-already-used.py <canon> <corpus-root-or-repo>
"""
import collections
import concurrent.futures as cf
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import canon_replay as harness

BIN, TARGET = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])


def scope_parts(convention):
    """(kind, directory, extension) for a convention's scope."""
    scope = convention["scope"]
    if not isinstance(scope, dict):
        return (scope, "", "")
    kind = next(iter(scope))
    value = scope[kind]
    if kind in ("DirExt", "DirChildrenExt"):
        return (kind, value[0], value[1])
    if kind == "Ext":
        return (kind, "", value)
    return (kind, value, "")


def speaks_for(convention, rel):
    kind, d, e = scope_parts(convention)
    ext = rel.rsplit(".", 1)[-1] if "." in os.path.basename(rel) else ""
    parent = rel.rsplit("/", 1)[0] if "/" in rel else ""
    if kind == "Repo":
        return True
    if kind == "Ext":
        return ext == e
    if kind == "Dir":
        return rel.startswith(d + "/")
    if kind == "DirExt":
        return ext == e and rel.startswith(d + "/")
    if kind == "DirChildrenExt":
        return ext == e and parent == d
    return False


def snapshot(data):
    d = os.path.join(data, "snapshots")
    with open(os.path.join(d, os.listdir(d)[0]), encoding="utf-8") as f:
        return json.load(f)


def audit(repo, cwd):
    data = harness.index(BIN, cwd, repo)
    naming = [c for c in snapshot(data)["conventions"] if c["id"].startswith("naming.")]
    blocking = [c for c in naming if c["enforcement"] == "Blocking"]

    by_ext = collections.defaultdict(list)
    for f in harness.tracked(cwd):
        by_ext[f.rsplit(".", 1)[-1] if "." in os.path.basename(f) else ""].append(f)

    cases = []
    for i, c in enumerate(blocking):
        _, d, ext = scope_parts(c)
        seen = set()
        for real in by_ext.get(ext, []):
            home, name = os.path.split(real)
            if home == d or name in seen:
                continue
            seen.add(name)
            if home.startswith(d + "/"):
                where = "inside"
            elif any(o["statement"] == c["statement"] and speaks_for(o, real) for o in naming):
                where = "declared"
            else:
                where = "elsewhere"
            cases.append((i, where, os.path.join(d, name), real))

    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        results = list(ex.map(lambda c: harness.inject(BIN, data, cwd, c[2], "x\n"), cases))

    refused = collections.defaultdict(lambda: collections.defaultdict(list))
    errors = []
    for (i, where, proposed, real), (verdict, why, _) in zip(cases, results):
        if verdict == "ERROR":
            errors.append((proposed, why))
        elif verdict == "DENY":
            refused[i][where].append((proposed, real))
    return blocking, len(cases), refused, errors


targets = harness.repos(TARGET) if os.path.isdir(os.path.join(TARGET, "realrepos")) \
    else [(os.path.basename(TARGET), TARGET)]

failed = []
for repo, cwd in targets:
    blocking, proposals, refused, errors = audit(repo, cwd)
    counts = {w: sum(1 for i in refused if w in refused[i]) for w in ("inside", "declared", "elsewhere")}
    print(f"  {repo:<14} {len(blocking):>4} blocking naming rules, {proposals:>6} proposals   "
          f"refusing a name used inside their own scope: {counts['inside']}, "
          f"under the same declared style: {counts['declared']}, elsewhere: {counts['elsewhere']}")
    for proposed, why in errors[:5]:
        print(f"      ERROR {proposed}: {why}")
        failed.append((repo, proposed, why))
    for i in sorted(refused):
        if "inside" not in refused[i]:
            continue
        proposed, real = refused[i]["inside"][0]
        print(f"      REFUSED {proposed}\n         {blocking[i]['id']} "
              f"({blocking[i]['agreeing']}/{blocking[i]['total']}), and the repository already has {real}")
        failed.append((repo, proposed, blocking[i]["id"]))

sys.exit(1 if failed else 0)
