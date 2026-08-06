#!/usr/bin/env python3
"""Count how often a Rails call passes a given argument, the way canon counts it.

Run this against a checkout to check whether a rule canon states, or fails to state, matches what
the code actually does. Every count it prints is reproducible from a repository and a SHA, and
nothing here reads canon's own snapshot, so a disagreement between the two is a real disagreement
rather than a tool reading its own output back.

It exists because the same nine facts were counted six different ways by six different ad hoc
greps, and the three method rules below are what the disagreements came down to. Each one changed
a headline number when it was got wrong:

1. Count TRACKED files with `git ls-files`, never `find`. A nested git worktree under .claude/
   doubled the WordPress PHP count from 134 to 268.
2. A caller is a file where the call name opens a logical line in ANY form, including bare with no
   arguments. Requiring a space or paren after the name missed 19 bare `has_paper_trail` callers
   and turned a 99/118 rule that fails its bar into a 99/99 rule that passes.
3. Strip comments and join continuation lines before matching. A commented-out call is not a
   caller, and a call whose arguments wrap is still one call.

Ratios here are RAW file counts. canon's agreement is recency-weighted (`recency_half_life_days`
defaults to 365), so a rule within a few points of its bar can move either way once weighted. The
bar itself is `Confidence::required_for`: 0.80 to 50 files, 0.85 to 500, 0.90 to 5,000, 0.95 above.

The nine facts in FACTS below are a Rails shape: they name calls that live in `db/migrate`,
`app/models` and `app/workers`. Add a row to measure another call, or another framework.

Usage: run from the root of the repository being measured.
    python3 measure.py            # print the table
    python3 measure.py --json     # machine-readable
"""

import json
import re
import subprocess
import sys


def tracked(prefix, ext=".rb"):
    """Files git tracks under prefix, with the given extension."""
    out = subprocess.run(
        ["git", "ls-files", prefix], capture_output=True, text=True, check=False
    ).stdout.split()
    return [f for f in out if f.endswith(ext)]


def head():
    return subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=False
    ).stdout.strip()


def logical_lines(path):
    """Source lines with comments stripped and continuations joined."""
    lines, buf = [], ""
    for raw in open(path, errors="ignore"):
        code = re.sub(r"#.*$", "", raw.rstrip("\n")).strip()
        if not code:
            continue
        buf = f"{buf} {code}".strip() if buf else code
        unbalanced = buf.count("(") > buf.count(")") or buf.count("{") > buf.count("}")
        if code.endswith((",", "\\", "(", "{", "[")) or unbalanced:
            continue
        lines.append(buf)
        buf = ""
    if buf:
        lines.append(buf)
    return lines


def conditional(files, call, key):
    """(files passing key, files calling call). The denominator is callers, not all files.

    A file that never makes the call is not evidence either way. Counting it as a dissenter is what
    buried `add_foreign_key(on_delete)` at 6% when its real agreement among callers is 92.5%.
    """
    callers = with_key = 0
    opens = re.compile(rf"^{re.escape(call)}(\s|\(|$)")
    passes = re.compile(rf"\b{re.escape(key)}\s*:")
    for path in files:
        hits = [line for line in logical_lines(path) if opens.match(line)]
        if not hits:
            continue
        callers += 1
        if any(passes.search(line) for line in hits):
            with_key += 1
    return with_key, callers


def required_for(sample):
    """Mirror of Confidence::required_for in crates/canon-core/src/confidence.rs."""
    if sample <= 50:
        return 0.80
    if sample <= 500:
        return 0.85
    if sample <= 5000:
        return 0.90
    return 0.95


FACTS = [
    ("add_foreign_key(on_delete)", "db/migrate", "add_foreign_key", "on_delete"),
    ("add_foreign_key(on_update)", "db/migrate", "add_foreign_key", "on_update"),
    ("add_index(algorithm)", "db/migrate", "add_index", "algorithm"),
    ("create_table(id)", "db/migrate", "create_table", "id"),
    ("has_paper_trail(skip)", "app/models", "has_paper_trail", "skip"),
    ("has_one(dependent)", "app/models", "has_one", "dependent"),
    ("sidekiq_options(queue)", "app/workers", "sidekiq_options", "queue"),
    ("sidekiq_options(retry)", "app/workers", "sidekiq_options", "retry"),
    ("sidekiq_throttle(concurrency)", "app/workers", "sidekiq_throttle", "concurrency"),
]


def main():
    sha = head()
    rows = []
    for label, prefix, call, key in FACTS:
        files = tracked(prefix)
        passing, callers = conditional(files, call, key)
        if not callers:
            rows.append({"rule": label, "scope": prefix, "derives": False, "reason": "no callers"})
            continue
        ratio = passing / callers
        bar = required_for(callers)
        rows.append(
            {
                "rule": label,
                "scope": prefix,
                "passing": passing,
                "callers": callers,
                "ratio": round(ratio, 4),
                "bar": bar,
                "derives": ratio >= bar,
            }
        )

    if "--json" in sys.argv:
        print(json.dumps({"sha": sha, "facts": rows}, indent=1))
        return

    print(f"HEAD {sha}   ratios are raw file counts, not recency-weighted\n")
    print(f"  {'rule':30s} {'passing/callers':>16s} {'ratio':>7s} {'bar':>6s}  derives")
    for r in rows:
        if not r.get("callers"):
            print(f"  {r['rule']:30s} {'-':>16s} {'-':>7s} {'-':>6s}  no ({r['reason']})")
            continue
        frac = f"{r['passing']}/{r['callers']}"
        mark = "yes" if r["derives"] else "NO"
        margin = abs(r["ratio"] - r["bar"])
        note = "  <- within 3 points of the bar" if margin < 0.03 else ""
        print(f"  {r['rule']:30s} {frac:>16s} {r['ratio']:6.1%} {r['bar']:6.2f}  {mark}{note}")


if __name__ == "__main__":
    main()
