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
  mutant   - a tracked file's type header rewritten to keep only its last
             base, so a directory where every class lists a mixin first
             (`class V(LoginRequiredMixin, ListView)`) gets a file that
             inherits the same real base without the mixin. A refusal here
             means a base-class rule learned the mixin's name instead.

The mutant only measures anything in a language that admits several bases at
once, because that is the only place "which one is the base" is a question. Two
are wired: Python, and Go, whose embedded fields are an unordered set.
"""
import os
import re
import sys, collections, concurrent.futures as cf

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import canon_replay as harness

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

# Rewrite a type header so the file keeps its directory's shape but varies the
# part a base-class rule reads. A repository whose views all read
# `class V(LoginRequiredMixin, ListView)` must not refuse `class V(ListView)`:
# the mixin is not the base, and the author cannot be told to add one.
#
# Only Python and Go are wired, because they are the only languages canon reads
# that put several bases in one ordered list. Ruby, PHP, TypeScript and
# JavaScript each name exactly one, so dropping all but the last is the
# identity and the case would measure nothing.
PY_CLASS = re.compile(r"^class\s+\w+\(", re.M)
GO_STRUCT = re.compile(r"^type\s+\w+\s+struct\s*\{[ \t]*$", re.M)
# An embedded field is a field declaration carrying a type and no name, which
# on its own line is a bare type and nothing else.
GO_EMBED = re.compile(r"^\s*\*?[A-Za-z_][\w.]*(\[[^\]]*\])?\s*$")

def py_header_span(content):
    """The `(...)` right after `class Name`, found by tracking bracket depth
    rather than matching up to the first `)`, so a call-expression base with
    its own parens or brackets does not truncate the scan early."""
    m = PY_CLASS.search(content)
    if m is None:
        return None
    open_idx = m.end() - 1
    depth = 0
    for i in range(open_idx, len(content)):
        c = content[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return open_idx, i
    return None                          # unbalanced; leave the file alone

def py_positional_bases(header):
    """Split on top-level commas, then drop anything that is not a plain
    positional base: a keyword argument (`metaclass=ABCMeta`) configures the
    class rather than naming a parent, the same reason `python.rs` already
    skips a `keyword_argument` node when it reads a class's base. A splat
    (`*bases`, `**kwargs`) is dropped for the same reason: it names no
    parent in particular."""
    items, depth, start = [], 0, 0
    for i, c in enumerate(header):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            items.append(header[start:i])
            start = i + 1
    items.append(header[start:])
    bases = []
    for item in items:
        s = item.strip()
        if not s or s.startswith("*"):
            continue
        depth = 0
        keyword = False
        for i, c in enumerate(s):
            if c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
            elif c == "=" and depth == 0:
                prev_c = s[i - 1] if i > 0 else ""
                next_c = s[i + 1] if i + 1 < len(s) else ""
                if next_c != "=" and prev_c not in "=!<>:":
                    keyword = True
                    break
        if not keyword:
            bases.append(s)
    return bases

def go_drop_leading_embeds(content):
    """Keep a struct's last embedded field and drop the ones before it.

    Go's embeds are an unordered set — `sync.Mutex` beside `BaseService` says
    the type is a service that also locks, in either order — so a file keeping
    only one of them is ordinary code and refusing it is a false positive."""
    m = GO_STRUCT.search(content)
    if m is None:
        return None
    lines = content.split("\n")
    start = content[:m.start()].count("\n")
    depth, embeds, end = 0, [], None
    for i in range(start, len(lines)):
        depth += lines[i].count("{") - lines[i].count("}")
        if depth == 0:
            end = i
            break
        if i > start and GO_EMBED.match(lines[i]):
            embeds.append(i)
    if end is None or len(embeds) < 2:
        return None
    dropped = set(embeds[:-1])
    return "\n".join(line for i, line in enumerate(lines) if i not in dropped)


def mutate(ext, content):
    """Drop every base but the last, keeping the file otherwise intact."""
    if ext == ".py":
        span = py_header_span(content)
        if span is None:
            return None
        open_idx, close_idx = span
        bases = py_positional_bases(content[open_idx + 1:close_idx])
        if len(bases) < 2:
            return None
        return content[:open_idx + 1] + bases[-1] + content[close_idx:]
    if ext == ".go":
        return go_drop_leading_embeds(content)
    return None

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
for repo, cwd in harness.repos(ROOT):
    data = harness.index(BIN, cwd, repo)
    cases = cases_for(cwd, harness.tracked(cwd))
    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        results = list(ex.map(lambda c: harness.inject(BIN, data, cwd, c[1], c[2]), cases))
    blocked = sum(1 for _, _, b in results if b)
    mutants = sum(1 for k, _, _ in cases if k == "mutant")
    bad = [(repo, k, rel, r) for (k, rel, _), r in zip(cases, results)
           if r[0] and k != "mutant"]
    mut = [(repo, k, rel, r) for (k, rel, _), r in zip(cases, results)
           if r[0] and k == "mutant"]
    grand["cases"] += len(cases)
    grand["blocked"] += blocked
    grand["mutants"] += mutants
    grand["bad"] += len(bad)
    grand["mutant"] += len(mut)
    problems += bad + mut
    print(f"  {repo:<14} {len(cases):>5} new files ({mutants:>4} mutants)   {blocked:>5} with a block   "
          f"{len(bad)} refused/errored   {len(mut)} base-order refusals")
print(f"\nTOTAL {grand['cases']} new files written into real directories "
      f"({grand['mutants']} of them mutants), {grand['blocked']} received a context block, "
      f"{grand['bad']} refused or errored, {grand['mutant']} refused for base order")
seen = set()
for repo, kind, rel, (verdict, why, _) in problems:
    key = (repo, why[:60])
    if key in seen:
        continue
    seen.add(key)
    print(f"  {verdict} [{kind}] {repo} {rel}\n      {why[:150]}")

# Without a snapshot every case comes back clean, which is what this harness
# reported for as long as it ran `inject` against an empty data directory. A
# block count of zero is that state, not a passing run.
if grand["blocked"] == 0:
    print("\nFAIL no case received a context block; the replay measured nothing")
    sys.exit(1)
if grand["mutants"] == 0:
    print("\nFAIL no mutant was generated; the base-order case measured nothing")
    sys.exit(1)
sys.exit(1 if problems else 0)
