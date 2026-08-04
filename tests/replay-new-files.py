"""Replay *new* files into real directories.

The existing-file replay cannot find a false positive by construction: every
file in the index has already voted, so a rule that refused it would not have
reached total agreement. Refusals happen to files that do not exist yet.

Two generators, both producing a file that a person would plausibly write:

  sibling  - the content of one tracked file at the path of another in the same
             directory. Both the name and the shape are real and idiomatic for
             that directory, so a refusal is a false positive.
  test     - a test file written into a directory that has blocking shape
             rules, in each of the common naming idioms.
  mutant   - a tracked file's class header rewritten to keep only its last
             base, so a directory where every class lists a mixin first
             (`class V(LoginRequiredMixin, ListView)`) gets a file that
             inherits the same real base without the mixin. A refusal here
             means a base-class rule learned the mixin's name instead.
"""
import os
import re
import json, subprocess, sys, os, collections, concurrent.futures as cf

BIN, ROOT = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
CODE = {".rb", ".js", ".ts", ".tsx", ".jsx", ".py", ".go", ".rs", ".php", ".vue", ".erb"}
TESTISH = ("spec", "test", "tests", "__tests__", "fixtures", "fixture")

def is_testish(rel):
    parts = rel.split("/")
    name = parts[-1]
    stem = name.rsplit(".", 1)[0]
    return (any(p in TESTISH for p in parts[:-1])
            or any(q in ("test", "tests", "spec", "test-d", "cy", "e2e", "stories")
                   for q in stem.split(".")[1:])
            or stem.endswith(("_spec", "_test", "_tests", "Test"))
            or stem.startswith("test_"))

def inject(cwd, rel, content):
    p = json.dumps({"session_id": "s", "cwd": cwd, "tool_name": "Write",
                    "tool_input": {"file_path": os.path.join(cwd, rel), "content": content}})
    r = subprocess.run([BIN, "inject"], input=p, capture_output=True, text=True, cwd=cwd)
    if r.returncode != 0 or r.stderr.strip():
        return ("ERROR", r.stderr[:160])
    h = json.loads(r.stdout or "{}").get("hookSpecificOutput", {})
    if h.get("permissionDecision") == "deny":
        why = h["permissionDecisionReason"].split("\n\n")
        return ("DENY", why[1].strip().replace("\n", " / ") if len(why) > 1 else "")
    return None

TEST_BODIES = {
    ".py":  "class Test{C}:\n    def test_works(self): pass\n\n    def test_fails(self): pass\n",
    ".rb":  "RSpec.describe {C} do\n  it 'works' do\n  end\nend\n",
    ".php": "<?php\nclass {C}Test extends TestCase\n{{\n    public function testWorks() {{}}\n\n    public function testFails() {{}}\n}}\n",
    ".ts":  "import {{ describe, it }} from 'vitest';\n\ndescribe('{C}', () => {{\n  it('works', () => {{}});\n}});\n",
    ".tsx": "import {{ describe, it }} from 'vitest';\n\ndescribe('{C}', () => {{\n  it('works', () => {{}});\n}});\n",
    ".js":  "describe('{C}', () => {{\n  it('works', () => {{}});\n}});\n",
    ".go":  "package {p}\n\nimport \"testing\"\n\nfunc TestWorks(t *testing.T) {{}}\n",
    ".rs":  "#[cfg(test)]\nmod tests {{\n    #[test]\n    fn works() {{}}\n}}\n",
}
TEST_NAMES = {
    ".py":  ["test_{s}.py", "{s}_test.py", "__tests__/test_{s}.py"],
    ".rb":  ["{s}_spec.rb", "{s}_test.rb"],
    ".php": ["{S}Test.php", "__tests__/{S}Test.php"],
    ".ts":  ["{s}.test.ts", "{s}.spec.ts", "__tests__/{s}.test.ts"],
    ".tsx": ["{s}.test.tsx", "{s}.spec.tsx"],
    ".js":  ["{s}.test.js", "{s}.spec.js"],
    ".go":  ["{s}_test.go"],
    ".rs":  ["{s}_test.rs"],
}

# Rewrite a class header so the file keeps its directory's shape but varies the
# part a base-class rule reads. A repository whose views all read
# `class V(LoginRequiredMixin, ListView)` must not refuse `class V(ListView)`:
# the mixin is not the base, and the author cannot be told to add one.
MUTATORS = {
    ".py":  (re.compile(r"^(class\s+\w+\()([^)]+)(\).*)$", re.M), ", "),
    ".rb":  (re.compile(r"^(class\s+[\w:]+\s*<\s*)([\w:]+)(.*)$", re.M), None),
    ".php": (re.compile(r"^(class\s+\w+\s+extends\s+)(\w+)(.*)$", re.M), None),
    ".ts":  (re.compile(r"^(export class\s+\w+\s+extends\s+)([\w.]+)(.*)$", re.M), None),
}

def mutate(ext, content):
    """Drop every base but the last, keeping the file otherwise intact."""
    entry = MUTATORS.get(ext)
    if entry is None:
        return None
    pattern, sep = entry
    m = pattern.search(content)
    if m is None:
        return None
    if sep is None:
        return None                      # single-base language: nothing to reorder
    bases = [b.strip() for b in m.group(2).split(sep) if b.strip()]
    if len(bases) < 2:
        return None
    kept = bases[-1]
    return content[:m.start()] + m.group(1) + kept + m.group(3) + content[m.end():]

def cases_for(cwd, files):
    by_dir = collections.defaultdict(list)
    for f in files:
        d, name = os.path.split(f)
        ext = os.path.splitext(name)[1]
        if ext in CODE and not is_testish(f):
            by_dir[(d, ext)].append(f)
    out = []
    for (d, ext), group in by_dir.items():
        group.sort()
        for i, src in enumerate(group[:6]):
            dst = group[(i + 1) % len(group)]
            if dst == src:
                continue
            try:
                content = open(os.path.join(cwd, src), encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            if len(content) > 200_000:
                continue
            out.append(("sibling", dst, content))
        for src in group[:6]:
            try:
                content = open(os.path.join(cwd, src), encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            if len(content) > 200_000:
                continue
            mutated = mutate(ext, content)
            if mutated is not None:
                out.append(("mutant", src, mutated))
        if ext in TEST_BODIES:
            base = os.path.basename(group[0]).rsplit(".", 1)[0].split(".")[0]
            cls = "".join(w.capitalize() for w in base.replace("-", "_").split("_")) or "Thing"
            body = TEST_BODIES[ext].format(C=cls, p=os.path.basename(d) or "main")
            for pattern in TEST_NAMES[ext]:
                rel = os.path.join(d, pattern.format(s=base, S=cls))
                out.append(("test", rel, body))
    return out

grand = collections.Counter()
problems = []
for repo in sorted(os.listdir(os.path.join(ROOT, "realrepos"))):
    cwd = os.path.join(ROOT, "realrepos", repo)
    if not os.path.isdir(cwd):
        continue
    files = [f for f in subprocess.run(["git", "ls-files"], cwd=cwd, capture_output=True,
                                       text=True).stdout.split("\n") if f]
    cases = cases_for(cwd, files)
    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        results = list(ex.map(lambda c: inject(cwd, c[1], c[2]), cases))
    bad = [(repo, k, rel, r) for (k, rel, _), r in zip(cases, results) if r and k != "mutant"]
    mut = [(repo, k, rel, r) for (k, rel, _), r in zip(cases, results) if r and k == "mutant"]
    grand["cases"] += len(cases)
    grand["bad"] += len(bad)
    grand["mutant"] += len(mut)
    problems += bad + mut
    print(f"  {repo:<14} {len(cases):>5} new files   {len(bad)} refused/errored   {len(mut)} base-order refusals")
print(f"\nTOTAL {grand['cases']} new files written into real directories, "
      f"{grand['bad']} refused or errored, {grand['mutant']} refused for base order")
seen = set()
for repo, kind, rel, (verdict, why) in problems:
    key = (repo, why[:60])
    if key in seen:
        continue
    seen.add(key)
    print(f"  {verdict} [{kind}] {repo} {rel}\n      {why[:150]}")
