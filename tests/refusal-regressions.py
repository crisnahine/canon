"""Every critical from the pre-push review, as an executable check."""
import os
import json, os, subprocess, sys, tempfile, shutil
BIN = os.path.abspath(sys.argv[1])
R = []
def mk(files, name):
    root = tempfile.mkdtemp(prefix=f"canon-{name}-")
    for rel, body in files.items():
        p = os.path.join(root, rel); os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").write(body)
    subprocess.run(["git","init","-q","."], cwd=root, check=True)
    subprocess.run(["git","add","-A"], cwd=root, check=True)
    subprocess.run(["git","-c","user.email=a@b","-c","user.name=t","commit","-qm","x"], cwd=root, check=True)
    data = tempfile.mkdtemp(prefix="canon-data-")
    subprocess.run([BIN,"index","--rebuild"], cwd=root, capture_output=True,
                   env={**os.environ,"CANON_DATA_DIR":data})
    return root, data
def inject(root, data, rel, content, tool="Write", extra=None):
    ti = {"file_path": os.path.join(root, rel)}
    ti.update(extra or {"content": content})
    p = json.dumps({"session_id":"s","cwd":root,"tool_name":tool,"tool_input":ti})
    r = subprocess.run([BIN,"inject"], input=p, cwd=root, capture_output=True, text=True,
                       env={**os.environ,"CANON_DATA_DIR":data})
    assert r.returncode == 0 and not r.stderr.strip(), f"fail-open breach: {r.returncode} {r.stderr[:200]}"
    h = json.loads(r.stdout or "{}").get("hookSpecificOutput", {})
    return h.get("permissionDecision") == "deny", h.get("permissionDecisionReason","")
def verify(root, data, rel, content, tool="Write"):
    # verify reads the file from disk rather than the payload, so it has to
    # exist there first; remove it after so the fixture is left as mk() built it.
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(content)
    try:
        p = json.dumps({"session_id":"s","cwd":root,"tool_name":tool,
                        "tool_input":{"file_path": path, "content": content}})
        r = subprocess.run([BIN,"verify"], input=p, cwd=root, capture_output=True, text=True,
                           env={**os.environ,"CANON_DATA_DIR":data})
        assert r.returncode == 0 and not r.stderr.strip(), f"fail-open breach: {r.returncode} {r.stderr[:200]}"
        h = json.loads(r.stdout or "{}").get("hookSpecificOutput", {})
        return h.get("additionalContext","")
    finally:
        os.remove(path)
def check(name, ok, detail=""): R.append((name, ok, detail))

# 1. Next.js / Nuxt / SvelteKit route filenames
files = {f"pages/{n}.tsx": "export default function P(){return null}\n"
         for n in ["user-profile","blog-post","about-page","contact-form","landing-hero"]}
root, data = mk(files, "routes")
for rel in ["pages/posts/[id].tsx","pages/[...slug].tsx","pages/_app.tsx"]:
    d, why = inject(root, data, rel, "export default function P(){return null}\n")
    check(f"route file {rel} not refused", not d, why[:110])
d, _ = inject(root, data, "pages/MyPage.tsx", "export default function P(){return null}\n")
check("a real style violation is still refused", d)
shutil.rmtree(root); shutil.rmtree(data)

files = {f"src/lib/{n}.ts": "export const x = 1\n" for n in
         ["userProfile","blogPost","aboutPage","contactForm","landingHero"]}
root, data = mk(files, "svelte")
for rel in ["src/routes/+page.server.ts","src/routes/blog/+server.ts","src/routes/+layout.ts"]:
    d, why = inject(root, data, rel, "export const x = 1\n")
    check(f"SvelteKit {os.path.basename(rel)} not refused", not d, why[:110])
shutil.rmtree(root); shutil.rmtree(data)

# 2. acronyms, digits, non-latin names in a cased directory
files = {f"src/{n}.tsx": "export const X = 1\n" for n in
         ["UserCard","BlogPost","AboutPage","ContactForm","LandingHero"]}
root, data = mk(files, "acronym")
for rel in ["src/SEO.tsx","src/FAQ.tsx","src/404.tsx"]:
    d, why = inject(root, data, rel, "export const X = 1\n")
    check(f"{os.path.basename(rel)} not refused in a PascalCase dir", not d, why[:110])
shutil.rmtree(root); shutil.rmtree(data)
files = {f"src/{n}.ts": "export const x = 1\n" for n in
         ["userProfile","blogPost","aboutPage","contactForm","landingHero"]}
root, data = mk(files, "cjk")
for rel in ["src/請求書.ts","src/404.ts"]:
    d, why = inject(root, data, rel, "export const x = 1\n")
    check(f"{os.path.basename(rel)} not refused in a camelCase dir", not d, why[:110])
shutil.rmtree(root); shutil.rmtree(data)

# 3. repo-wide .md rule derived in docs/ must not govern .github/
files = {f"docs/{n}.md": "# x\n" for n in
         ["getting-started","api-reference","contributing-guide","release-notes","style-guide","faq-page"]}
root, data = mk(files, "repowide")
d, why = inject(root, data, ".github/PULL_REQUEST_TEMPLATE.md", "## What\n")
check("repo-wide md rule does not refuse .github/PULL_REQUEST_TEMPLATE.md", not d, why[:130])
d, _ = inject(root, data, "docs/BadName.md", "# x\n")
check("the same rule still refuses inside the directory it was counted in", d)
shutil.rmtree(root); shutil.rmtree(data)

# 4. CSS modules must not govern a plain stylesheet
files = {f"src/{n}.module.css": ".a{}\n" for n in
         ["Button","Card","Modal","Header","Footer","Sidebar"]}
root, data = mk(files, "cssmod")
d, why = inject(root, data, "src/globals.css", ".a{}\n")
check("a CSS-module rule does not refuse a plain stylesheet", not d, why[:130])
shutil.rmtree(root); shutil.rmtree(data)

# 5. a file the index skipped (over 512 KB) must not be refused
big = "class BulkImporter < ApplicationService\n  def call; end\n  def also; end\nend\n" + "# pad\n" * 90000
files = {f"app/services/{n}.rb": f"class {n.title().replace('_','')} < ApplicationService\n  def call; end\nend\n"
         for n in ["charge_card","refund_payment","settle_batch","send_receipt","void_invoice","apply_credit"]}
files["app/services/bulk_importer.rb"] = big
root, data = mk(files, "oversize")
d, why = inject(root, data, "app/services/bulk_importer.rb", big)
check("an oversized tracked file is not refused", not d, why[:130])
shutil.rmtree(root); shutil.rmtree(data)

# 6. Rust: foreign impl target, and a private helper module
# Each sample file has a base type too, so `shape.base` exists and a foreign
# impl wrongly setting `superclass` is actually detectable.
files = {f"src/handlers/{n}.rs":
         f"use std::fmt;\npub struct {n.title()};\nimpl {n.title()} {{ pub fn call(&self) {{}} }}\n"
         f"impl fmt::Display for {n.title()} {{ fn fmt(&self) {{}} }}\n"
         for n in ["alpha","beta","gamma","delta","epsilon","zeta"]}
root, data = mk(files, "rustimpl")
# The foreign impl comes FIRST, so a last-segment match would take its trait as
# the base before the local one is ever seen.
d, why = inject(root, data, "src/handlers/theta.rs",
    "use std::{fmt, io};\npub struct Error;\n"
    "impl From<Error> for io::Error { fn from(_: Error) -> Self { unimplemented!() } }\n"
    "impl Error { pub fn call(&self) {} }\n"
    "impl fmt::Display for Error { fn fmt(&self) {} }\n")
check("a foreign impl target does not add methods or a base to a local type", not d, why[:160])
# The stem names no declared type, so `primary_type` must fall through to the
# surface comparison the fix alters rather than being saved by a name match.
d, why = inject(root, data, "src/handlers/run_it.rs",
    "use std::fmt;\npub struct Iota;\nimpl Iota { pub fn call(&self) {} }\n"
    "impl fmt::Display for Iota { fn fmt(&self) {} }\n"
    "mod helper { pub struct Big; impl Big { pub fn a(&self){} pub fn b(&self){} pub fn c(&self){} } }\n")
check("a private helper module is not the file's subject", not d, why[:160])
# And the mirror: a root marker type must not hide the module that has the
# surface, or the file resolves to something with nothing on it.
d, why = inject(root, data, "src/handlers/sealed_up.rs",
    "use std::fmt;\npub struct Sealed;\n"
    "pub mod inner { use std::fmt; pub struct Theta; impl Theta { pub fn call(&self) {} }\n"
    "impl fmt::Display for Theta { fn fmt(&self) {} } }\n")
check("a root marker type does not hide the module that has the surface", not d, why[:160])
shutil.rmtree(root); shutil.rmtree(data)

# --- a repository-wide rule whose sample spans two directories ---------------
files = {f"docs/{n}.md": "# x\n" for n in ["getting-started","api-reference","quick-start"]}
files.update({f"website/{n}.md": "# x\n" for n in ["install-guide","upgrade-guide","tuning-guide"]})
files["src/main.ts"] = "export const x = 1\n"
root, data = mk(files, "twotops")
for rel in [".github/PULL_REQUEST_TEMPLATE.md", ".github/ISSUE_TEMPLATE/bug_report.md",
            "ARCHITECTURE.md", "packages/core/API_NOTES.md"]:
    d, why = inject(root, data, rel, "# x\n")
    check(f"a two-directory sample does not govern {rel}", not d, why[:130])
d, _ = inject(root, data, "docs/BadName.md", "# x\n")
check("and still refuses inside a directory that did vote", d)
# The content-less branch has to agree with the one that has content.
for tool, extra in [("NotebookEdit", {"notebook_path": os.path.join(root, ".github/PULL_REQUEST_TEMPLATE.md"), "new_source": "x"}),
                    ("Edit", {"file_path": os.path.join(root, ".github/PULL_REQUEST_TEMPLATE.md"), "old_string": "absent", "new_string": "y"})]:
    p = json.dumps({"session_id":"s","cwd":root,"tool_name":tool,"tool_input":extra})
    r = subprocess.run([BIN,"inject"], input=p, cwd=root, capture_output=True, text=True,
                       env={**os.environ,"CANON_DATA_DIR":data})
    denied = json.loads(r.stdout or "{}").get("hookSpecificOutput",{}).get("permissionDecision")=="deny"
    check(f"{tool} agrees with Write on the same path", not denied)
shutil.rmtree(root); shutil.rmtree(data)

# --- an acronym is unclassifiable in every style, not only PascalCase --------
files = {f"docs/{n}.md": "# x\n" for n in
         ["getting-started","api-reference","quick-start","style-guide","release-notes","tuning-guide"]}
root, data = mk(files, "acronym-kebab")
for rel in ["docs/FAQ.md","docs/API.md","docs/SEO.md"]:
    d, why = inject(root, data, rel, "# x\n")
    check(f"{rel} is not refused by a kebab-case rule", not d, why[:130])
d, _ = inject(root, data, "docs/MyNewDoc.md", "# x\n")
check("a real style violation is still refused", d)
shutil.rmtree(root); shutil.rmtree(data)

# --- nothing canon reads may hang or be unbounded ----------------------------
import stat
files = {f"app/services/{n}.rb": f"class {n.title().replace('_','')} < ApplicationService\n  def call; end\nend\n"
         for n in ["charge_card","refund_payment","settle_batch","send_receipt","void_invoice","apply_credit"]}
root, data = mk(files, "hazards")
fifo = os.path.join(root, "app/services/pipe.rb")
os.mkfifo(fifo)
for cmd, ti in [("inject", {"file_path": fifo, "old_string": "a", "new_string": "b"}),
                ("verify", {"file_path": fifo, "content": "x"})]:
    p = json.dumps({"session_id":"s","cwd":root,"tool_name":"Edit","tool_input":ti})
    try:
        r = subprocess.run([BIN,cmd], input=p, cwd=root, capture_output=True, text=True,
                           env={**os.environ,"CANON_DATA_DIR":data}, timeout=10)
        check(f"{cmd} does not hang on a FIFO", r.returncode == 0 and not r.stderr.strip())
    except subprocess.TimeoutExpired:
        check(f"{cmd} does not hang on a FIFO", False, "timed out")
os.remove(fifo)
cfg = os.path.join(root, ".canon.toml")
os.mkfifo(cfg)
p = json.dumps({"session_id":"s","cwd":root,"tool_name":"Write",
                "tool_input":{"file_path": os.path.join(root,"app/services/x.rb"),"content":"class X; end\n"}})
try:
    r = subprocess.run([BIN,"inject"], input=p, cwd=root, capture_output=True, text=True,
                       env={**os.environ,"CANON_DATA_DIR":data}, timeout=10)
    check("inject does not hang on a .canon.toml that is a FIFO", r.returncode == 0)
except subprocess.TimeoutExpired:
    check("inject does not hang on a .canon.toml that is a FIFO", False, "timed out")
os.remove(cfg)
shutil.rmtree(root); shutil.rmtree(data)

# --- round three: what a third review found in the second round's fixes ----

# 1+2. Rust: trait-impl methods are not the type's own surface
files = {f"src/handlers/{n}.rs":
         f"use std::fmt;\npub struct {n.title()};\nimpl {n.title()} {{ pub fn call(&self) {{}} }}\n"
         f"impl fmt::Display for {n.title()} {{ fn fmt(&self) {{}} }}\n"
         for n in ["alpha","beta","gamma","delta","epsilon","zeta"]}
root, data = mk(files, "rust-surface")
d, w = inject(root, data, "src/handlers/theta.rs",
  "use std::fmt;\npub struct Sealed;\nimpl fmt::Display for Sealed { fn fmt(&self) {} }\n"
  "pub mod inner { use std::fmt; pub struct Theta; impl Theta { pub fn call(&self) {} }\n"
  "impl fmt::Display for Theta { fn fmt(&self) {} } }\n")
check("a marker with a Display impl does not hide the module with the surface", not d, w[:150])
d, w = inject(root, data, "src/handlers/client.rs",
  "use std::fmt;\n#[derive(Debug)] pub enum Error { NotFound }\n"
  "impl fmt::Display for Error { fn fmt(&self) {} }\nimpl std::error::Error for Error {}\n"
  "pub mod client { pub struct Client; impl Client { pub fn call(&self) {} } }\n")
check("a unit error type with the required impls does not become the subject", not d, w[:150])
d, w = inject(root, data, "src/handlers/iota.rs",
  "use std::fmt;\npub struct Iota;\nimpl Iota { pub fn call(&self) {} }\n"
  "impl fmt::Display for Iota { fn fmt(&self) {} }\n"
  "mod helper { pub struct Big; impl Big { pub fn a(&self){} pub fn b(&self){} } }\n")
check("a private helper module is still not the subject", not d, w[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 3. a bare acronym must not delete the naming rule when deriving
files = {f"docs/{n}.md": "# x\n" for n in
         ["getting-started","api-reference","quick-start","style-guide","release-notes","tuning-guide"]}
files["docs/FAQ.md"] = "# x\n"
root, data = mk(files, "acronym-derive")
out = subprocess.run([BIN,"explain","docs/"], cwd=root, capture_output=True, text=True,
                     env={**os.environ,"CANON_DATA_DIR":data}).stdout
check("one acronym does not delete the naming rule", "kebab-case" in out, out[:120])
shutil.rmtree(root); shutil.rmtree(data)

# 4. TS/JS: a companion class declared second must not become the subject
files = {f"src/components/{n}.tsx": f"export class {n} extends BaseComponent {{ render(): void {{}} }}\n"
         for n in ["UserCard","OrderList","PayoutForm","LoginPanel","NavBar","SideMenu"]}
root, data = mk(files, "companion")
d, w = inject(root, data, "src/components/PriceTag.tsx",
  "export class PriceTag extends BaseComponent { render(): void {} }\n"
  "export class PriceTagStore extends Store { update(): void {} }\n")
check("a companion class declared second is not the subject", not d, w[:150])
d, _ = inject(root, data, "src/components/BadOne.tsx",
  "export class BadOne extends Other { nope(): void {} }\n")
check("a genuine base violation is still refused", d)
shutil.rmtree(root); shutil.rmtree(data)

# 5. sample coverage below the top level, and for a DirExt scope
files = {f"src/components/{n}.tsx": "export const X = 1;\n" for n in
         ["UserCard","OrderList","PayoutForm","LoginPanel","NavBar","SideMenu"]}
files.update({f"web/{n}.tsx": "export const X = 1;\n" for n in
              ["home-page","about-page","help-page","legal-page","terms-page","press-page"]})
root, data = mk(files, "subdirs")
for rel in ["src/hooks/useToggle.tsx","src/pages/about-us.tsx","src/utils/date_fmt.tsx"]:
    d, w = inject(root, data, rel, "export const X = 1;\n")
    check(f"a sample in src/components does not govern {rel}", not d, w[:130])
d, _ = inject(root, data, "src/components/bad_name.tsx", "export const X = 1;\n")
check("and still refuses inside the directory it was counted in", d)
shutil.rmtree(root); shutil.rmtree(data)

# 6. Python: a dotted or generic base class
files = {f"app/services/{n}.py":
         f"from app import base\n\n\nclass {''.join(w.title() for w in n.split('_'))}(base.BaseService):\n"
         f"    def execute(self):\n        pass\n"
         for n in ["charge_card","refund_payment","settle_batch","send_receipt","void_invoice","apply_credit"]}
root, data = mk(files, "py-base")
out = subprocess.run([BIN,"explain","app/services/"], cwd=root, capture_output=True, text=True,
                     env={**os.environ,"CANON_DATA_DIR":data}).stdout
check("a dotted base class is read as the base", "BaseService" in out, out[:150])
d, w = inject(root, data, "app/services/new_thing.py",
  "from app import base\n\n\nclass NewThing(base.BaseService[Order]):\n    def execute(self):\n        pass\n")
check("a generic base class is not refused as having none", not d, w[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 7. Rust: the order of impl blocks must not decide a refusal
files = {f"src/net/{n}.rs":
         f"use std::fmt;\npub struct {n.title()};\nimpl fmt::Display for {n.title()} {{ fn fmt(&self) {{}} }}\n"
         f"impl {n.title()} {{ pub fn call(&self) {{}} }}\n"
         for n in ["alpha","beta","gamma","delta","epsilon","zeta"]}
root, data = mk(files, "impl-order")
d, w = inject(root, data, "src/net/theta.rs",
  "use std::fmt;\npub struct Theta;\nimpl std::error::Error for Theta {}\n"
  "impl fmt::Display for Theta { fn fmt(&self) {} }\nimpl Theta { pub fn call(&self) {} }\n")
check("impl order does not decide the base check", not d, w[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 8+9. Ruby: def self.call, and a rooted superclass
files = {f"app/services/{n}.rb":
         f"class {''.join(w.title() for w in n.split('_'))} < ApplicationService\n"
         f"  def self.call(x)\n    new(x).call\n  end\n\n  def call\n    run\n  end\n\n"
         f"  private\n\n  def run; end\nend\n"
         for n in ["charge_card","refund_payment","settle_batch","send_receipt","void_invoice","apply_credit"]}
root, data = mk(files, "ruby-self")
out = subprocess.run([BIN,"explain","app/services/"], cwd=root, capture_output=True, text=True,
                     env={**os.environ,"CANON_DATA_DIR":data}).stdout
check("a class-method service derives a shape rule", "ApplicationService" in out, out[:150])
d, w = inject(root, data, "app/services/new_thing.rb",
  "module Billing\n  class NewThing < ::ApplicationService\n    def self.call(x)\n      new(x).call\n    end\n\n"
  "    def call\n      run\n    end\n\n    private\n\n    def run; end\n  end\nend\n")
check("a rooted ::Base is not refused by a rule naming the same class", not d, w[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 10. an extra method is advice, not a refusal
files = {f"db/migrate/2024010{i}_add_{n}.rb":
         f"class Add{n.title()} < ActiveRecord::Migration[7.1]\n  def change\n  end\nend\n"
         for i, n in enumerate(["one","two","three","four","five","six"], start=1)}
root, data = mk(files, "migration")
d, w = inject(root, data, "db/migrate/20240707_backfill_totals.rb",
  "class BackfillTotals < ActiveRecord::Migration[7.1]\n  def up\n  end\n\n"
  "  def down\n    raise ActiveRecord::IrreversibleMigration\n  end\nend\n")
check("an up/down migration is not refused", not d, w[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 11+12. canon's own reads must not hang
files = {f"app/services/{n}.rb": f"class {n.title()} < ApplicationService\n  def call; end\nend\n"
         for n in ["alpha","beta","gamma","delta","epsilon","zeta"]}
root, data = mk(files, "state-hazards")
snap = [f for f in os.listdir(os.path.join(data,"snapshots"))]
os.remove(os.path.join(data,"snapshots",snap[0])); os.mkfifo(os.path.join(data,"snapshots",snap[0]))
try:
    d, _ = inject(root, data, "app/services/x.rb", "class X; end\n")
    check("a FIFO at the snapshot path does not hang inject", True)
except subprocess.TimeoutExpired:
    check("a FIFO at the snapshot path does not hang inject", False, "timed out")
os.remove(os.path.join(data,"snapshots",snap[0]))
os.makedirs(os.path.join(data,"sessions"), exist_ok=True)
for f in os.listdir(os.path.join(data,"sessions")): os.remove(os.path.join(data,"sessions",f))
shutil.rmtree(root); shutil.rmtree(data)

# --- round four: the base-order bug, Django mixin-first inheritance --------

# 1. a directory where every view lists its access mixin before the real base
files = {f"shop/views/p{i}.py":
         "from django.contrib.auth.mixins import LoginRequiredMixin\n"
         "from django.views.generic import ListView\n\n\n"
         f"class P{i}ListView(LoginRequiredMixin, ListView):\n"
         "    def get(self, request):\n        return None\n"
         for i in range(1, 7)}
root, data = mk(files, "django-view-mixin")
public_view = ("from django.views.generic import ListView\n\n\n"
               "class PublicListView(ListView):\n    def get(self, request):\n        return None\n")
d, w = inject(root, data, "shop/views/public.py", public_view)
check("a view that keeps the directory's real base without the login mixin is not refused", not d, w[:150])
# A negative assertion alone can pass on a verify that went silent for the
# wrong reason, so it needs a positive control in the same fixture: an
# unrelated base must still be reported, or the gate could pass by accident.
gate = verify(root, data, "shop/views/public.py", public_view)
check("the view advisory reports no base mismatch for the real base", "inherits from" not in gate, gate[:150])
odd_view = "class Odd(SomethingElse):\n    def get(self, request):\n        return None\n"
liveness = verify(root, data, "shop/views/odd.py", odd_view)
check("the view advisory still reports a base mismatch for an unrelated base", "inherits from" in liveness, liveness[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 2. the same shape on a model directory: a mixin ahead of models.Model
files = {f"shop/models/p{i}.py":
         "from shop.models.base import TimeStamped\n"
         "from django.db import models\n\n\n"
         f"class P{i}(TimeStamped, models.Model):\n"
         "    def clean(self):\n        pass\n"
         for i in range(1, 7)}
root, data = mk(files, "django-model-mixin")
plain_model = ("from django.db import models\n\n\n"
               "class Plain(models.Model):\n    def clean(self):\n        pass\n")
d, w = inject(root, data, "shop/models/plain.py", plain_model)
check("a model that keeps the directory's real base without the timestamp mixin is not refused", not d, w[:150])
gate = verify(root, data, "shop/models/plain.py", plain_model)
check("the model advisory reports no base mismatch for the real base", "inherits from" not in gate, gate[:150])
odd_model = "class OddModel(SomethingElse):\n    def clean(self):\n        pass\n"
liveness = verify(root, data, "shop/models/odd.py", odd_model)
check("the model advisory still reports a base mismatch for an unrelated base", "inherits from" in liveness, liveness[:150])
shutil.rmtree(root); shutil.rmtree(data)

# 3. Ruby, single base: shape.base enforcement is not disabled outright, only
# narrowed for the language where "first base" is ambiguous. A directory that
# agrees on one base with no ordering question must still refuse a stranger.
files = {f"app/services/{n}.rb":
         f"class {n.title().replace('_','')} < ApplicationService\n  def call; end\nend\n"
         for n in ["charge_card","refund_payment","settle_batch","send_receipt","void_invoice","apply_credit"]}
root, data = mk(files, "ruby-single-base")
d, _ = inject(root, data, "app/services/odd_one.rb",
  "class OddOne < SomethingElse\n  def call; end\nend\n")
check("a Ruby class with an unrelated base is still refused", d)
shutil.rmtree(root); shutil.rmtree(data)

# --- round five: a React hook is inside the style system, unlike a route file
# `useOnboarding` is valid camelCase, so `outside_the_style_system` never
# caught it; the framework mandates the `use` prefix, not the team, and the
# file could not be renamed to satisfy a PascalCase directory's rule.
files = {f"src/components/UniversalOnboarding/components/{n}.tsx":
         f"export const {n} = () => null;\n"
         for n in ["EmailState","StepInitial","StepNew","StepPassword","StepSuccess"]}
root, data = mk(files, "react-hook")
d, w = inject(root, data, "src/components/UniversalOnboarding/components/useOnboarding.tsx",
  "export const useOnboarding = () => null;\n")
check("a React hook is not refused in a PascalCase component directory", not d, w[:150])
for rel in ["src/components/UniversalOnboarding/components/step-two.tsx",
            "src/components/UniversalOnboarding/components/onboarding_helpers.tsx"]:
    d, _ = inject(root, data, rel, "export const X = () => null;\n")
    check(f"a genuinely mis-styled name is still refused: {os.path.basename(rel)}", d)
shutil.rmtree(root); shutil.rmtree(data)

# The same defect a second way: `not-found.tsx` and `global-error.tsx` are the
# App Router's own names for a route boundary, and the only hyphenated members
# of its special-file set, so they read as an ordinary two-word kebab-case
# name in a PascalCase directory instead of a name nothing chose.
files = {f"app/dashboard/{n}.tsx": f"export const {n} = () => null;\n"
         for n in ["UserCard","OrderList","PayoutForm","LoginPanel","NavBar"]}
root, data = mk(files, "app-router-specials")
for rel in ["not-found", "global-error"]:
    d, w = inject(root, data, f"app/dashboard/{rel}.tsx", "export default function P(){return null}\n")
    check(f"App Router {rel}.tsx is not refused in a PascalCase directory", not d, w[:150])
d, _ = inject(root, data, "app/dashboard/bad-name.tsx", "export const X = () => null;\n")
check("a real style violation is still refused", d)
shutil.rmtree(root); shutil.rmtree(data)

# --- round six: Go embeds are an unordered set, not an ordered base list ----
# Six structs embedding `sync.Mutex` before `BaseService` derived "types here
# inherit from `sync.Mutex`" at 6/6 and denied a struct embedding `BaseService`
# alone. Which embed lands in `superclass` is an artefact of source order, the
# same reason Rust and Python were already exempt.
files = {f"internal/svc/s{i}.go":
         "package svc\n\nimport \"sync\"\n\n"
         f"type S{i} struct {{\n\tsync.Mutex\n\tBaseService\n\tname string\n}}\n\n"
         f"func (s *S{i}) Run() {{}}\n"
         for i in range(1, 7)}
root, data = mk(files, "go-embeds")
subset = ("package svc\n\ntype Plain struct {\n\tBaseService\n\tname string\n}\n\n"
          "func (s *Plain) Run() {}\n")
d, w = inject(root, data, "internal/svc/plain.go", subset)
check("a Go struct embedding a different subset is not refused", not d, w[:150])
out = subprocess.run([BIN,"explain","internal/svc/"], cwd=root, capture_output=True, text=True,
                     env={**os.environ,"CANON_DATA_DIR":data}).stdout
base = out.split("shape.base")[1].split("\n\n")[0] if "shape.base" in out else ""
check("the Go base rule survives as advice rather than vanishing",
      "Advisory" in base and "Blocking" not in base, base[:150])
odd = ("package svc\n\ntype Odd struct {\n\tSomethingElse\n\tname string\n}\n\n"
       "func (s *Odd) Run() {}\n")
liveness = verify(root, data, "internal/svc/odd.go", odd)
check("the Go advisory still reports a base mismatch for an unrelated embed",
      "inherits from" in liveness, liveness[:150])
shutil.rmtree(root); shutil.rmtree(data)

# --- a typing marker read as the base type ---------------------------------

# The mirror of the Django case above. Python's frameworks order a base list
# mixins-first, so the last entry is the type the class is; its typing markers
# go last for the same reason, and the two conventions collide. Six files of
# `class Page(BaseModel, Generic[T])` stated that types here inherit from
# `Generic`. Advisory, so what is wrong is the sentence rather than a refusal.
for marker, extra in [("Generic[T]", "from typing import Generic, TypeVar\n\nT = TypeVar('T')\n"),
                      ("Protocol", "from typing import Protocol\n"),
                      ("ABC", "from abc import ABC\n")]:
    files = {f"app/schemas/p{i}.py":
             f"from app.schemas.base import BaseModel\n{extra}\n\n"
             f"class P{i}(BaseModel, {marker}):\n    def render(self):\n        return None\n"
             for i in range(1, 7)}
    root, data = mk(files, "py-marker")
    out = subprocess.run([BIN, "explain", "app/schemas/"], cwd=root, capture_output=True, text=True,
                         env={**os.environ, "CANON_DATA_DIR": data}).stdout
    base = out.split("shape.base")[1].split("\n\n")[0] if "shape.base" in out else ""
    check(f"a directory using {marker} states its real base, not the marker",
          "`BaseModel`" in base, base[:150])
    # A positive control: the marker must not resurface as composition either,
    # or the mixin family states the same wrong thing in its own words.
    check(f"{marker} is not restated as an included module",
          "Types here include" not in out, out[:200])
    # And the check has to agree with the statement, or the rule derives and
    # never fires.
    conforming = ("from app.schemas.base import BaseModel\n\n\n"
                  "class Plain(BaseModel):\n    def render(self):\n        return None\n")
    gate = verify(root, data, "app/schemas/plain.py", conforming)
    check(f"a {marker}-free file on the real base reports no base mismatch",
          "inherits from" not in gate, gate[:150])
    odd = "class Odd(SomethingElse):\n    def render(self):\n        return None\n"
    liveness = verify(root, data, "app/schemas/odd.py", odd)
    check(f"the {marker} directory still reports a mismatch for an unrelated base",
          "inherits from" in liveness, liveness[:150])
    shutil.rmtree(root); shutil.rmtree(data)

ok = sum(1 for _, o, _ in R if o)
for n, o, d in R:
    if not o: print(f"  FAIL {n}\n       {d}")
print(f"\n{ok}/{len(R)} review criticals fixed")
sys.exit(0 if ok == len(R) else 1)
