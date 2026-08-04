"""One check per open canon issue. Run against any build; prints PASS/FAIL."""
import os
import json, os, subprocess, sys, tempfile, shutil

BIN, REPOS = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
DATA = tempfile.mkdtemp(prefix="canon-issues-")
ENV = {**os.environ, "CANON_DATA_DIR": DATA}

def run(root, *args, env=None):
    return subprocess.run([BIN, *args], cwd=root, capture_output=True, text=True,
                          env={**ENV, **(env or {})})

def index(name):
    root = os.path.join(REPOS, name)
    run(root, "index", "--rebuild")
    return root

def hook(root, cmd, payload, env=None):
    r = subprocess.run([BIN, cmd], input=json.dumps(payload), cwd=root,
                       capture_output=True, text=True, env={**ENV, **(env or {})})
    assert r.returncode == 0, f"{cmd} exited {r.returncode}: {r.stderr[:200]}"
    assert not r.stderr.strip(), f"{cmd} wrote to stderr: {r.stderr[:200]}"
    return json.loads(r.stdout or "{}").get("hookSpecificOutput", {})

def write_payload(root, rel, content, tool="Write"):
    return {"session_id": "s", "cwd": root, "tool_name": tool,
            "tool_input": {"file_path": os.path.join(root, rel), "content": content}}

def edit_payload(root, rel, old, new):
    return {"session_id": "s", "cwd": root, "tool_name": "Edit",
            "tool_input": {"file_path": os.path.join(root, rel),
                           "old_string": old, "new_string": new}}

def denied(h):  return h.get("permissionDecision") == "deny"
def reason(h):  return h.get("permissionDecisionReason", "")
def context(h): return h.get("additionalContext", "")

RESULTS = []
def check(issue, name, ok, detail=""):
    RESULTS.append((issue, name, ok, detail))

BAD_PY = "class VoidSubscription:\n    def perform(self): pass\n\n    def rollback(self): pass\n"

# ---- #9 Edit reaches file states Write refuses -------------------------------
root = index("services")
h = hook(root, "inject", write_payload(root, "app/services/void_subscription.py", BAD_PY))
check(9, "Write of a violating file is refused", denied(h), reason(h)[:80])
# an Edit that turns a conforming file into a violating one
target = "app/services/charge_card.py"
h = hook(root, "inject", edit_payload(root, target, "def execute(self, payload):",
                                      "def perform(self, payload):"))
check(9, "Edit that breaks the entrypoint rule is refused", denied(h), reason(h)[:120])
# an Edit that keeps the file conforming must still be allowed
h = hook(root, "inject", edit_payload(root, target, "return self._run(payload)",
                                      "return self._run(dict(payload))"))
check(9, "Edit that keeps the file conforming is allowed", not denied(h), reason(h)[:120])
# a naming violation needs no content at all
h = hook(root, "inject", {"session_id": "s", "cwd": root, "tool_name": "Edit",
                          "tool_input": {"file_path": os.path.join(root, "app/services/BadName.py"),
                                         "old_string": "a", "new_string": "b"}})
check(9, "naming rule is enforced without content", denied(h), reason(h)[:100])

# ---- #10 Ext-scoped Blocking rules refuse writes they can never state --------
root = index("extonly")
h = hook(root, "inject", write_payload(root, "lib/tasks/seven_backfill.rake", "task :x do\nend\n"))
stated = bool(context(h))
h2 = hook(root, "inject", write_payload(root, "lib/tasks/seven-backfill-v2.rake", "task :x do\nend\n"))
check(10, "a rule that can refuse is stated first", stated or not denied(h2),
      f"compliant said {'something' if stated else 'nothing'}; violating {'refused' if denied(h2) else 'allowed'}")

# ---- #11 refusal wording and denominator -------------------------------------
root = index("denominator")
h = hook(root, "inject", write_payload(root, "docs/some-note.txt", "x\n"))
if denied(h):
    text = reason(h)
    check(11, "refusal does not claim 'this directory' for a repo-wide rule",
          "this directory" not in text, text.split("\n")[0][:120])
    check(11, "refusal names the scope it counted over",
          "**/*.txt" in text or "matching" in text, [l for l in text.split("\n") if l.startswith("-")][:1])
else:
    check(11, "refusal wording", True, "no refusal to word")
# the files the sample excluded must not be refused
for rel in ["LICENSE.txt", "spec/fixtures/ips-v7.txt"]:
    h = hook(root, "inject", write_payload(root, rel, "x\n"))
    check(11, f"excluded file {rel} is not refused", not denied(h), reason(h)[:100])

# ---- #12 verify branches for the three unchecked families --------------------
root = index("families")
def verify_written(rel, content):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(content)
    try:
        return context(hook(root, "verify", write_payload(root, rel, content)))
    finally:
        os.remove(path)
out = verify_written("spec/services/new_thing_spec.rb",
                     "require 'spec_helper'\n\nRSpec.describe NewThing do\nend\n")
check(12, "a wrong import is reported by verify", "import" in out.lower(), out[:160])
out = verify_written("spec/services/new_thing_test.rb",
                     "require 'rails_helper'\n\nRSpec.describe NewThing do\nend\n")
check(12, "a wrong test suffix is reported by verify", "_spec" in out or "named" in out.lower(), out[:160])
out = verify_written("app/services/untested_thing.rb",
                     "require 'application_service'\n\nclass UntestedThing < ApplicationService\n  def call; end\nend\n")
check(12, "a missing test is reported by verify", "test" in out.lower(), out[:160])

# ---- #13 .canon.toml takes effect without a rebuild --------------------------
root = index("services")
cfg = os.path.join(root, ".canon.toml")
def with_config(text, env=None):
    if text is None:
        if os.path.exists(cfg): os.remove(cfg)
    else:
        open(cfg, "w").write(text)
    try:
        return hook(root, "inject", write_payload(root, "app/services/nope.py", BAD_PY), env=env)
    finally:
        if os.path.exists(cfg): os.remove(cfg)
check(13, "baseline refuses", denied(with_config(None)))
check(13, "enforce = false takes effect at once", not denied(with_config("enforce = false\n")))
check(13, "CANON_ENFORCE=0 takes effect at once",
      not denied(with_config(None, env={"CANON_ENFORCE": "0"})))
ids = [l.split("[")[-1].rstrip("]") for l in reason(with_config(None)).split("\n") if l.startswith("- ") and "[" in l]
check(13, "the refusal names the ids to suppress", len(ids) >= 1, ids)
if ids:
    check(13, "suppressing those ids takes effect at once",
          not denied(with_config("suppress = " + json.dumps(ids) + "\n")), ids)
check(13, "an unparseable config does not keep enforcement on",
      not denied(with_config("enforce = false\nmin_fils = 9\n")))
check(13, "a duplicated suppress key does not keep enforcement on",
      not denied(with_config('suppress = ["a"]\nsuppress = ["b"]\n')))

# ---- #14 suppressing a rule must not unmask an equivalent one ----------------
root = index("nested")
h = hook(root, "inject", write_payload(root, "api/some-note.txt", "x\n"))
first = [l.split("[")[-1].rstrip("]") for l in reason(h).split("\n") if l.startswith("- ") and "[" in l]
if first:
    open(os.path.join(root, ".canon.toml"), "w").write("suppress = " + json.dumps(first) + "\n")
    run(root, "index", "--rebuild")
    h2 = hook(root, "inject", write_payload(root, "api/some-note.txt", "x\n"))
    check(14, "suppressing the rule does not re-derive it under a new id",
          not denied(h2), f"suppressed {first}, now {reason(h2)[:120]}")
    os.remove(os.path.join(root, ".canon.toml"))
    run(root, "index", "--rebuild")
else:
    check(14, "suppression", True, "nothing refused to suppress")

# ---- #15 explain filters by its path argument --------------------------------
root = index("mixed")
out = run(root, "explain", "lib/tasks/one_backfill.rake").stdout
scopes = [l.split()[-1] for l in out.split("\n") if l.strip().startswith("scope")]
check(15, "explain <file> shows only rules that apply", all(s.endswith(".rake") or s == "this repository" for s in scopes), scopes)
out = run(root, "explain", "data/").stdout
scopes = [l.split()[-1] for l in out.split("\n") if l.strip().startswith("scope")]
check(15, "explain <dir> shows only rules that apply", all(".rb" not in s and ".rake" not in s for s in scopes), scopes)

# ---- #16 the header does not claim languages that contributed nothing --------
root = index("quiet")
h = hook(root, "session-start", {"session_id": "s", "cwd": root})
line = context(h)
check(16, "header does not credit a language that derived nothing",
      "Python" not in line or "0 conventions" in line, line.strip()[:140])

# ---- #17 one defect is stated once, however many scopes derived the rule ----
root = index("nested2")
bad = "class Widget < WrongBase\n  def one; 1; end\n  def two; 2; end\n  def three; 3; end\nend\n"
rel = "app/services/billing/invoices/widget.rb"
p = os.path.join(root, rel)
os.makedirs(os.path.dirname(p), exist_ok=True)
open(p, "w").write(bad)
h = hook(root, "verify", write_payload(root, rel, bad))
lines = [l for l in context(h).split("\n") if l.startswith("- ")]
base = [l for l in lines if "inherits from" in l]
check(17, "one line per defect, not one per nesting level", len(base) <= 1, lines)
check(17, "the line kept names the narrowest scope that derived it",
      not base or "invoices" in base[0], base)
os.remove(p)

# ---- #18 a naming rule speaks for its own scope root ------------------------
# Every sampled file lives in a subdirectory, so the scope root is an ancestor
# of all of them and used to answer neither way.
root = index("scoperoot")
deep = hook(root, "inject", write_payload(root, "src/components/group/subone/zz-probe.tsx", "export const A = 1;\n"))
at_root = hook(root, "inject", write_payload(root, "src/components/zz-probe.tsx", "export const A = 1;\n"))
check(18, "the rule refuses in a directory it sampled", denied(deep), reason(deep)[:100])
check(18, "and at the root of its own scope, which it never sampled directly",
      denied(at_root), reason(at_root)[:100] or "allowed")

# ---- #19 one version-numbered vendor file cannot pin a naming rule ----------
root = index("vendored")
out = run(root, "index", "--rebuild").stdout
check(19, "a vendored version number witnesses no style", out.strip().startswith("0 conventions"), out.strip()[:120])
h = hook(root, "inject", write_payload(root, "js/myWidget.js", "var a = 1;\n"))
check(19, "and nothing is refused for the style it never witnessed", not denied(h), reason(h)[:120])

# ---- #20 raising the floor states strictly less -----------------------------
root = index("floorsplit")
def count(root):
    return int(run(root, "index", "--rebuild").stdout.split()[0])
def blocking(root):
    return run(root, "explain").stdout.count("enforcement Blocking")
default, default_block = count(root), blocking(root)
open(os.path.join(root, ".canon.toml"), "w").write("confidence_floor = 1.0\n")
strict, strict_block = count(root), blocking(root)
os.remove(os.path.join(root, ".canon.toml"))
check(20, "a higher floor never states more", strict <= default, f"{strict} against {default}")
check(20, "nor produces more rules that may refuse a write",
      strict_block <= default_block, f"{strict_block} against {default_block}")
open(os.path.join(root, ".canon.toml"), "w").write("confidence_floor = 0.6\n")
out = run(root, "index", "--rebuild").stdout + run(root, "index", "--rebuild").stderr
check(20, "a floor the hard floor already covers is a load error",
      "must be between" in out, out.strip()[:140])
os.remove(os.path.join(root, ".canon.toml"))
run(root, "index", "--rebuild")

# ---- #21 an unchanged tree derives the same set every time ------------------
# Two equal-sized children under a parent that derives nothing itself, so the
# rollup has a genuine tie to break: which child names it, and which twelve of
# its sixteen files become the evidence.
root = index("rolluptie")
# Probabilistic on the broken build rather than certain: the tie resolved on a
# per-process hash seed, so each rebuild was a coin flip and eight of them miss
# the defect under one time in a hundred.
seen = set()
for _ in range(8):
    run(root, "index", "--rebuild")
    seen.add(run(root, "explain").stdout)
check(21, "eight rebuilds of one tree produce one answer", len(seen) == 1,
      f"{len(seen)} distinct outputs")

# ---- #16 the new families reach the languages that had no vocabulary --------
root = index("views")
out = run(root, "explain").stdout
check(16, "an ERB view tree derives its format segment", "*.html.erb" in out, out[:200])
# `verify` reads the file from disk rather than the payload, because for an
# `Edit` the payload carries a fragment and not the resulting file.
missing = os.path.join(root, "app/views/orders/show.erb")
open(missing, "w").write("<h1>x</h1>\n")
h = hook(root, "verify", write_payload(root, "app/views/orders/show.erb", "<h1>x</h1>\n"))
check(16, "and a view missing it is told", "*.html.erb" in context(h), context(h)[:200])
os.remove(missing)

shutil.rmtree(DATA, ignore_errors=True)
by_issue = {}
for issue, name, ok, detail in RESULTS:
    by_issue.setdefault(issue, []).append((name, ok, detail))
failed = 0
for issue in sorted(by_issue):
    checks = by_issue[issue]
    bad = [c for c in checks if not c[1]]
    failed += len(bad)
    print(f"#{issue}: {len(checks) - len(bad)}/{len(checks)} pass")
    for name, ok, detail in checks:
        if not ok:
            print(f"   FAIL {name}\n        {detail}")
print(f"\n{len(RESULTS) - failed}/{len(RESULTS)} checks pass")
sys.exit(1 if failed else 0)
