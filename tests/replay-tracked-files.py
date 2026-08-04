"""Replay every tracked file back through the write path.

A file already in the index has voted on every rule that speaks for it, so a
rule that refuses it is a rule that contradicts its own evidence. Nothing here
may be refused, and nothing here may break the fail-open contract.
"""
import concurrent.futures as cf
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import canon_replay as harness

BIN, ROOT = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
EXTS = {".rb", ".rake", ".gemspec", ".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx",
        ".py", ".pyi", ".go", ".rs", ".php", ".vue", ".erb", ".rhtml", ".toml", ".yml", ".yaml",
        ".json", ".md", ".scss", ".css", ".html", ".stub", ".rst", ".adoc"}


def check(binary, data, cwd, rel):
    try:
        content = open(os.path.join(cwd, rel), encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    if len(content) > 400_000:
        return None
    return harness.inject(binary, data, cwd, rel, content)


grand_files = 0
grand_blocked = 0
grand_bad = []
for repo, cwd in harness.repos(ROOT):
    data = harness.index(BIN, cwd, repo)
    files = [f for f in harness.tracked(cwd) if os.path.splitext(f)[1] in EXTS]
    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        results = list(ex.map(lambda f: check(BIN, data, cwd, f), files))
    judged = [(f, r) for f, r in zip(files, results) if r is not None]
    blocked = sum(1 for _, (_, _, b) in judged if b)
    bad = [(repo, f) + r[:2] for f, r in judged if r[0] is not None]
    grand_files += len(judged)
    grand_blocked += blocked
    grand_bad += bad
    print(f"  {repo:<14} {len(judged):>5} files   {blocked:>5} with a block   {len(bad)} refused/failed")

print(f"\nTOTAL {grand_files} tracked files replayed through the write path, "
      f"{grand_blocked} received a context block, {len(grand_bad)} refused or errored")
for b in grand_bad[:60]:
    print("  ", b[0], b[1], b[2], "|", b[3].strip()[:150])

# A run where nothing received a block proves only that no rule was loaded.
# That is the state a missing snapshot leaves the harness in, and it reported
# a clean sweep from it for as long as it existed.
if grand_blocked == 0:
    print("\nFAIL no case received a context block; the replay measured nothing")
    sys.exit(1)
sys.exit(1 if grand_bad else 0)
