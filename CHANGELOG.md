# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.1] — 2026-08-03

Everything here was found by running the release binary against fourteen real
repositories — Mastodon, Laravel, RuboCop, Nuxt, Vue, Redux Toolkit, ripgrep,
Flask, requests, gin, cobra, Slim, Sinatra, axios — and twenty-four
purpose-shaped ones covering the idioms each supported language actually uses.

### Fixed

- **`.canon.toml` did nothing until the next session.** `enforce`, `suppress`
  and `CANON_ENFORCE` were baked into the snapshot at derivation, and the hot
  path read the stored decision. So the escape hatch a refusal points at was
  inert at the one moment it was needed: you were blocked, you turned
  enforcement off, and you were blocked again. Enforcement and suppression are
  now resolved per write.
- **A refusal named the rule in prose but not by id**, and `suppress` is keyed
  by id. Against the running host the model inferred three plausible ids, wrote
  them into `.canon.toml`, and was refused again. The refusal now prints the
  ids and the `suppress` line ready to paste.
- **A naming rule could come from one name repeated.** Ten files called
  `Cargo.toml` derived "files here are named in `PascalCase`" at total
  agreement, enforced — which refused an ordinary `crates/x/deny.toml`. In
  canon's own repository and in ripgrep. The sample is now counted in distinct
  names, not files.
- **A multi-part file name was compatible with no style at all**, because a `.`
  is not a word character. One `globals.d.ts` took a directory of six
  `snake_case` files from four conventions to none; every Rails `*.html.erb`
  and every Angular or NestJS `*.service.ts` was silently excluded. Style is
  now read from the name root, the part before the first dot.
- **`__init__.py` made a naming rule impossible for any Python package.** A
  leading underscore matches no style, so one dunder file broke the directory.
  Dunder names describe a role the language assigns, and are excluded like
  `README` already was.
- **Non-ASCII file names were compatible with no style**, so a blocking rule
  refused `café_service.py` in a repository whose other names happened to be
  ASCII. Style classification is Unicode-aware.
- **Colocation counted unrelated files as tests.** `src/**/.gitattributes` in
  Laravel came out at "every file here has a test of the same name (37/37)",
  because a dotfile has an empty name and empty matched empty; `composer.json`
  came out at 37/39 because one fixture under `tests/` shared the name. Pairing
  is now keyed by name *and* extension, empty names are excluded, and data
  files are not asked whether they have a test.
- **Python could not see a decorated class or function.** `@dataclass`,
  `@app.route`, `@pytest.fixture`, `@admin.register` — the wrapper node was
  skipped. Identical repositories derived five conventions plain and two
  decorated.
- **Go lost every method of a generic type.** The receiver of
  `func (c *Cache[T]) Get()` never matched the declared `Cache`.
- **Rust lost every method of a generic type**, for the same reason: `impl<W>
  Printer<W>` names `Printer<W>`. Eighty-one of ripgrep's two hundred and sixty
  top-level impls are generic. Rust also could not see anything inside an
  inline `mod`, and now skips `#[cfg(test)] mod tests` rather than counting
  fixtures as the file's shape.
- **A `..` in the target path was matched literally**, so
  `app/services/../../vendor/x.rb` still started with `app/` and had a service
  object's rules applied to a vendored file. Paths are normalised lexically,
  and a relative `file_path` is resolved against the working directory instead
  of being ignored.
- **A rolled-up rule could refuse a write.** It is assembled from the rules of
  child directories and generalises to siblings that never voted, which is
  exactly where a refusal is wrong. A rolled-up ancestor also no longer absorbs
  the child it was built from, which had quietly moved directories from
  enforced to advisory.
- **`canon check` and `canon explain` read a different directory from the
  hooks.** The host passes `CLAUDE_PLUGIN_DATA` to a hook and nothing to a
  terminal, so the audit surface a refusal sends you to reported "no snapshot
  yet" for a repository the session had already indexed.
- **A test file was refused for not being shaped like the code it tests.** A
  colocated `test_void_invoice.py` written into a directory of service objects
  was told it must inherit `BaseService` and expose one public method, because
  the rule was still at total agreement — the counterexample was the file being
  written. Shape rules no longer refuse a test.
- **Two Rust files were refused by an arity rule they satisfy.** Making inline
  `mod` bodies visible attached every `impl` to whichever type was declared
  first, so two modules declaring the same name, and a type implemented from
  two `#[cfg]`-gated modules, both came out as one type with two methods. An
  `impl` now resolves to the nearest declaration it can see, and a repeated
  method name counts once — a type cannot expose two methods under one name in
  any build, so a repeat is a conditional alternative or an overload signature.
- **A `.canon.toml` that would not parse silently turned enforcement back on.**
  The two likeliest ways to get that file wrong are a typo in a key and a
  second `suppress =` line appended to a file that already has one; both are
  parse errors, both happen while trying to turn a refusal off, and both were
  answered by refusing again with nothing printed. An unloadable config now
  degrades to `enforce = false`.
- `canon explain` reports a suppressed rule as `Suppressed` rather than
  `Advisory`. It is the surface a refusal sends you to in order to confirm the
  suppression took, and a suppressed rule is not downgraded, it is gone.

### Changed

- `inject` costs 2.6 ms again on a 9,870-file repository, down from 27.3 ms.
  Compiling a tree-sitter query happens once per process and a hook is one
  process per write, so the whole 25 ms landed on every Ruby write — for call
  and raise facts that the check then discarded. The write path extracts
  structure only.
- Python's `test_*.py` is recognised as a test. `pytest` collects it by
  default, so a Python repository derived no test-naming rule at all. Prefix
  matching is Python-only: applied everywhere it read Go's `test_helpers.go`
  and a `spec_runner.rake` task as tests.
- `*.test-d.tsx`, `*.cy.ts`, `*.e2e.ts` and `*.stories.tsx` are tests. Five
  `*.test-d.tsx` files were the only `.tsx` in Vue's repository and derived an
  enforced rule that every `.tsx` is camelCase.
- A repository-wide naming rule is worth stating on its own. Withholding it
  left a Go repository whose only rule was unanimous across all 58 files saying
  nothing before a write. How tests are named is now withheld instead, unless
  the file being written is one.
- `canon explain` and `canon check` report enforcement as it would be applied
  now, not as it was recorded, and `canon check` prints the data directory in
  use.
- `SNAPSHOT_VERSION` is 7. A snapshot from before this holds naming rules
  derived from one repeated name, rules a suppression should have removed, and
  rules derived while a framework-named file was read as a style violation —
  all of them enforced.

### Fixed — the open issues

Triaged by reproducing each against the build rather than reading it. Two were
already closed by the work above (#10, #13); the rest were not.

- **`Edit` reached file states `Write` refused** (#9). The deny check read the
  `Write`-only `content` field, so `Edit` and `NotebookEdit` never reached it:
  the same path, the same resulting bytes, opposite outcomes, and `Edit` is the
  tool a model reaches for once a file exists. The result is now reconstructed
  from disk plus `old_string` → `new_string`, so the check sees the file that
  will land. When the result cannot be known, the path-only rules still apply,
  because a naming rule never reads the content.
- **`canon explain` ignored its path argument** (#15). Asking about a `.rake`
  file listed the rule for `**/*.csv`. A query naming a file now uses the same
  predicate injection does; a query naming a directory admits a
  repository-wide extension rule only where the evidence puts one. Anything
  able to refuse a write sorts first, because a refusal is what sends people
  here.
- **Suppressing a rule re-derived it under a new id** (#14). Suppression ran
  before the pass that removes a narrower copy of a rule its ancestor already
  states, so suppressing `naming.repo.txt` produced `naming.api.txt` and the
  same refusal. It now runs last.
- **Three derived statement forms had no check** (#12): imports, test naming,
  and colocation were stated before a write and never checked after one.
  Imports are the highest-value family canon derives and were the least
  checked. The import check uses the query-derived list the rule was counted
  from; `inject` still does not, because no import rule can refuse a write.
- **A refusal claimed "every file in this directory" while counting the whole
  repository** (#11). Every count now renders the scope it was counted over,
  so the sentence and the number describe the same set.
- **The header credited languages that derived nothing** (#16). `languages` is
  read off the conventions rather than off the files walked, so a Rails
  repository no longer reports ERB when both its conventions are Ruby's.
- **ERB derived nothing at all on idiomatic Rails code** (#16). A leading
  underscore is compatible with no style and `shared_styles` is all-or-nothing,
  so one `_form.html.erb` silenced an entire view tree — and every Rails view
  tree has partials. The same fault took the naming rule off `requests`' own
  package directory, over `_internal_utils.py`. A leading underscore is a role
  marker every framework spends on the same idea, and never a style.
- **An unreadable `.canon.toml` silently re-enabled enforcement.** Found by
  adversarial review of the fix above, not by the issue. `read_to_string`
  failing was treated as "no config file", which means the enforcing defaults,
  so a config saved as UTF-16 — what PowerShell's `>` writes, on a platform
  canon supports — carried `enforce = false` and refused the write anyway, with
  no `INVALID` line on `canon check` and no rebuild that would help. Absent and
  unreadable are now different answers.
- **`bin/install-local` deleted the binary the installed plugin asks for.** It
  named the copy after the source version, so bumping ahead of the last release
  left the cached plugin looking for a version that was no longer there,
  downloading the released binary, and running that. An hour of fixes measured
  as having no effect. A copy now goes in for every version an installed plugin
  might ask for.


### Fixed — what a pre-push review found

Six lenses over the outgoing diff, every finding then handed to a separate
agent whose job was to refute it. Eight survived as critical, and one root
cause accounts for three of them: the fix above for `_form.html.erb` was
written for a leading underscore rather than for the class of name it belongs
to.

- **Every file-based router's own file names were refused.** `[id].tsx`,
  `[...slug].tsx`, `pages/[id].vue`, `+page.server.ts`, `+layout.ts`. A name
  containing a character no style admits is compatible with nothing, and the
  check read "compatible with nothing" as "breaks the rule" — so Next.js, Nuxt
  and SvelteKit route files were denied, and the developer cannot rename them.
  The guard is now the general one: a name outside the style system is
  unclassifiable, not wrong, in both the sample and the check.
- **An acronym, a digit or a non-Latin script was refused in a cased
  directory.** `SEO.tsx` and `FAQ.tsx` in a `PascalCase` components directory,
  `404.tsx`, `請求書.ts` in a `camelCase` one. A separator-free all-caps name is
  now Pascal-compatible, which is how a Pascal-cased project writes an acronym,
  and a name that distinguishes no style cannot break one.
- **A rule refused files its sample never covered.** Six `.md` files in `docs/`
  derived a repository-wide rule that refused `.github/PULL_REQUEST_TEMPLATE.md`;
  a directory of `Button.module.css` derived one that refused a plain
  `globals.css`. A refusal now requires the evidence to cover the qualifier and,
  for a repository-wide rule, the top-level directory.
- **A file the index skipped was refused on every edit.** Anything over 512 KB
  never votes, so no rule was counted over it, and a tracked 630 KB service
  object was held to rules it had never been allowed to break — contradicting
  the README's central safety claim in the one case that could reach it.
- **Rust attributed foreign `impl` blocks to local types.** `impl LocalTrait for
  io::Error` was reduced to its last path segment and merged into whatever
  `Error` the file declared, inventing methods on a type that has none. A
  qualified path now only matches when it is rooted in this file.
- **A private helper module could become the file's subject.** Making inline
  `mod` bodies visible put nested types in the same flat list as top-level ones,
  and the subject is chosen by largest surface. A nested type is now only the
  subject when the file declares none at the root.
- **`resulting_file` read any path with no type or size guard**, on the write
  path. A FIFO in the tree would have hung the hook forever, which is the one
  failure a fail-open harness cannot disguise.
- **`has_test_for` walked the filesystem with no exclusions**, so a
  `node_modules` exhausted its entry budget and the exhaustion was reported as
  "no test found" — a confident claim produced by giving up. It now skips the
  same directories the fallback walk does, bounds its depth, and says nothing
  when it runs out of budget.
- **A broken config disabled enforcement and the diagnostic for it at once.**
  The log level came from the same fallback, so the line explaining why could
  not be written. `CANON_LOG` now resolves independently of whether the config
  loads.
- **`bin/install-local` deleted every binary and installed none** when the
  version line did not parse.

### Verified

- Every one of the eight open issues has a check in a harness that runs against
  any build: 23 assertions, all passing, each failing on the build before this.
- Every critical from the review has one too: 18 assertions covering four
  frameworks' route conventions, acronyms and non-Latin names, scope-versus-
  sample mismatches, oversized files, and both Rust attribution defects.
- 15,265 tracked files from the fourteen repositories replayed through the
  write path: 11,299 given conventions, none refused, none errored.
- 5,700 *new* files written into those same directories — one file's content at
  its neighbour's name, and a test in each naming idiom into every directory
  with an enforceable rule — none refused. This is the harness that matters:
  every file already in the index has voted, so the first replay confirms the
  invariant but cannot find a false positive. All three above were found here
  and nowhere else.
- Against the running host, with the plugin installed: asked for a service and
  told nothing about house style, the first write matched all five derived
  conventions. Asked for one that broke three rules, the write did not happen,
  and the model used the `suppress` line the refusal printed and succeeded on
  the retry — in the same session.

## [0.4.0] — 2026-08-03

### Added

- Import conventions. "Files here import from `@tanstack/react-query`" is the
  highest-value thing canon can say and the one it had no word for: a wrong
  import compiles, type-checks, and passes review whenever a plausible
  alternative exists. Counted per file rather than per occurrence, so a barrel
  import cannot decide a directory, and relative paths are excluded because
  `./thing` names something different from every directory. (#7)
- Colocation. "Every file here has a test of the same name", matched by stem
  rather than by path shape, because `spec/` mirrors `app/` in some
  repositories and `__tests__/` sits beside the file in others. (#7)
- Vue and ERB, through `Parser::set_included_ranges`. The buffer is parsed by
  the grammar of the language canon has conventions about, restricted to the
  ranges that language occupies, and the tree keeps the original offsets so a
  reported line still points at the right line. 340 `.erb` files that were
  invisible now contribute. Every language canon knows is now wired.

### Changed

- The injected block says nothing when everything that applies is a
  repository-wide fallback. It used to announce conventions for a component
  directory and then offer one rule about test files, for a file that is not a
  test: a header claiming coverage the body does not deliver is worse than
  silence. (#7)
- `SNAPSHOT_VERSION` is 4.

## [0.3.0] — 2026-08-03

### Fixed

- A file that followed every convention exactly was reported for breaking two,
  and once those rules reached total agreement it was refused outright.
  Verifying judged every declared type while deriving resolved one, so a
  namespace module counted as a type with no base and no public methods. Both
  halves now resolve the subject the same way, in one place. (#1)
- The entrypoint-name rule was withheld from any type with more than one public
  method, so the files that broke it hardest were told the least, and fixing
  one file took two round trips. (#2)
- Moving the working directory into a nested repository silently switched to a
  snapshot that was never built, and every hook went quiet for the rest of the
  session. The snapshot is now found by searching upward, and a root with none
  says so once. (#3)
- `reconcile` compared a written file only against tracked files, so several
  near-identical files written in the same turn were invisible to each other —
  which is the case worth catching. The session's own ledger is now part of the
  index. (#4)
- Naming rules were derived for binary assets. Twelve of them on one workspace,
  including 2,912 `.jpg` files, each taking a slot in a 1,500-character budget.
  (#8)
- Five conventionally-named files at a repository root produced "files here are
  named in `SCREAMING_SNAKE_CASE`" for every `.md` in the project. (#8)
- Required agreement now rises with the size of the sample. The widest scopes
  were clearing the floor by the smallest margin and generating the most
  messages: one rule covered every `.rb` file in a Rails API at 0.81 and
  complained about a migration written the way the framework asks. (#8)

### Added

- A rule is stated at the parent when every child directory that has an opinion
  holds the same one. Agreement is counted over directories, not files, because
  a tree where twelve subdirectories each hold a rule without exception can
  still sit at 0.82 across its files. A new subdirectory now inherits the rule
  instead of getting nothing. (#6)
- `enforce`, on by default, with `CANON_ENFORCE` to turn it off. Refusing a
  write is the only channel a model cannot decline; the other three were each
  measured being declined, and a tool that only advises is followed when
  convenient. The safety is in what qualifies rather than in the switch: a rule
  may only refuse when every file in scope already agrees, which makes a false
  positive a contradiction rather than a risk. Replaying 400 tracked Ruby files
  from a production repository through the write path refused none. (#5)
- File-name casing can refuse, when enabled. It is the safest check there is:
  a string comparison on the path, with no parsing and no question about which
  type in the file is the subject. (#5)

### Changed

- The report after a write asks for the change rather than restating the
  policy. (#5)
- `SNAPSHOT_VERSION` is 3. A snapshot from before this release holds the
  over-broad rules, none of the rolled-up ones, and marks every convention
  advisory, so an upgraded install would refuse nothing until its commit
  changed.
- `injection_budget` is 3,000, up from 1,500. Headroom rather than a fix:
  measured on a 13,456-file workspace, real paths spend 262 to 486 bytes and
  raising the ceiling to 4,000 produced byte-identical output. What bounds the
  block is how many conventions exist for a path, not how many fit.



### Added

- canon can refuse a write. `Enforcement::Blocking` existed from the first
  release and nothing ever constructed it; it is now derived for the three
  rules whose check cannot be wrong about a legitimate file — public method
  count, entrypoint name, base type — and only where the repository agrees
  without a single exception.
- Stale state is swept on the cold path. A snapshot is written per repository
  and nothing removed it when that repository was deleted; a touched-file list
  is removed when a turn ends, and a session that is killed never ends.

### Why refusing, and not something gentler

Advisory context is ignorable, which was measured rather than assumed. A hook
demanded a marker line the model had no other reason to write:

- context before the write reaches the model in time and steers it
- a `PostToolUse` block delivered its reason and the turn ended anyway
- a `Stop` block held the turn open three times and the edit still never came
- refusing the write is the only channel that does not need the model to agree

`PreToolUse` also accepts an `updatedInput`, and it works: a hook can replace
the content before it is written. canon does not use it. Silently authoring
someone's code from a rule derived by counting is a worse failure than the one
it would prevent.

## [0.2.0] — 2026-08-03

### Added

- A declarative extraction layer. Call sites, raises and imports are matched by
  tree-sitter queries in `queries/<language>/facts.scm` rather than by another
  hand-rolled walk per language. Adding a fact to a language is now editing an
  `.scm` file.
- Layering conventions, listed as unbuilt until now: who a directory talks to.
  On a real worker directory, `Files here call ``User``. (9/10, 0.90)`. It is
  the kind of rule no linter checks and every team holds.
- The capture vocabulary is canon's and identical across languages. Every
  grammar crate exports a `TAGS_QUERY`, and they disagree: Go's reports
  `@reference.type` where Ruby's reports `@reference.call`, and TypeScript's
  covers only the constructs TypeScript adds, on the assumption the JavaScript
  query is concatenated with it.

### Changed

- `rust-version` is 1.97, the current release. canon ships prebuilt binaries for
  eight targets, so the floor only affects building from source, and the CI job
  that proves it moves with it.
- A file is parsed once and read twice. Extractors previously each parsed their
  own tree, so a second pass would have parsed the file again.
- `SNAPSHOT_VERSION` is 2. A snapshot built before the query layer holds no
  layering rules, so it is discarded rather than kept until the commit changes.
- `toml` moves from 0.9 to 1. Its error for an unknown key now lists the keys
  that are valid, which is the message `canon check` shows a human.

### Removed

- Absence-of-raising as a convention. It was built, measured against a
  9,546-file repository, and produced six rules covering `spec/`, `vendor/`,
  `config/`, `db/` and all of `app/` — every one arithmetic rather than a choice
  anyone made.

## [0.1.2] — 2026-08-03

### Fixed

- Windows on ARM had no binary at all, and neither did any musl system. The
  release built five of the eight triples the shim can compute, so those
  platforms got a 404 and then the fail-open path, which produces no symptom
  beyond a plugin that never speaks.
- The shim now asks the loader whether the C library is musl or glibc, instead
  of assuming glibc and handing Alpine something that cannot run.

### Added

- All eight targets are built natively rather than cross-compiled. The
  tree-sitter grammars are C, and a missing cross toolchain yields something
  unrunnable rather than a build failure. Each build job now runs what it built.
- A release smoke stage: every published asset is downloaded on its own
  platform, checked against `SHA256SUMS`, and executed. Building an artifact
  and publishing a working one are different claims.
- `tests/asset-coverage.sh`, which enumerates the shim's computable triples and
  the release's published assets from source and fails on any mismatch. That
  mismatch is what shipped a Windows plugin that did nothing, and it is
  structural rather than a typo, so it needed a test rather than vigilance.
- CI runs tests and the resolution shim on both architectures of all three
  platforms, the fail-open contract on all three, and the Windows batch wrapper
  through `cmd.exe`, which is the path the host actually takes there.

## [0.1.1] — 2026-08-03

### Fixed

- Windows never got a binary. The release publishes
  `canon-x86_64-pc-windows-msvc.exe`, and the resolution shim asked for the
  same name without the suffix, so the download 404'd. Had it succeeded it
  would have saved the file under a name Windows refuses to execute. The
  Windows wrapper had the mirror-image fault: it looked for `canon.exe` while
  the shim installs a versioned `canon-<version>.exe`, so the two never met.
  Every path failed open, which is why the only symptom was a plugin that
  silently did nothing.
- Installing a binary now removes the one it replaces, so the Windows wrapper's
  `canon-*` match is unambiguous.

### Added

- A `shim` CI job that runs the resolution shell under bash on Linux, macOS and
  Windows and requires it to find a binary. Nothing tested the shim before, and
  it is where every platform difference lives.
- A check that the asset name the shim requests matches the one the release
  workflow publishes, so the two cannot drift apart again.

## [0.1.0] — 2026-08-03

First release. A Claude Code plugin that derives a repository's conventions
from its own code and states them before each file is written.

### Added

- Five crates with boundaries that hold three properties: derivation rules are
  testable without a repository or a grammar, adding a language cannot reach
  those rules, and exactly two functions in the workspace may write to a
  standard stream.
- Tier 0 conventions from paths alone — naming style, test-file naming,
  canonical exemplars. Works on any text repository in any language.
- Tier 1 conventions from tree-sitter — public surface size, entrypoint name,
  base type, module export count. Wired for Ruby, JavaScript, JSX, TypeScript,
  TSX, Python, Go, Rust and PHP, spanning six different visibility models.
- `inject` on `PreToolUse`: the conventions for the file about to be written.
- `verify` on `PostToolUse`: how the written file differs from them.
- `reconcile` on `Stop`: near-duplicate detection across the turn's files.
- `subagent-start`: the same manifest into a worker with an empty context
  window, which is the reason this is a plugin and not a skill.
- `canon check`, which prints the capability table read from the binary, so
  documentation cannot claim a language the build does not link.
- `canon explain`, the audit surface: every rule with the files behind it, and
  suppression by id in `.canon.toml`.
- Layered configuration, file-only logging, and concrete error enums per crate.
- `tests/fail-open.sh`: 75 hostile-payload checks against the shipped binary,
  asserting exit 0, silence on stderr, and stdout empty or valid JSON.
- `tests/injection-reaches-the-model.sh`: verifies against the installed host
  that `PreToolUse` context reaches the model before the tool runs.
- CI across Linux, macOS and Windows, a minimum-supported-Rust job at 1.85, and
  a job asserting the manifests agree with the crate version.

### Notes from building it

Five defects survived a green test suite and were found only by pointing the
tool at production repositories and hostile input. They are recorded in
`STATUS.md` because each one is the reason a piece of the design looks the way
it does:

- The filesystem walk found 498,419 files where git tracks 5,445. The file list
  comes from `git ls-files` now.
- `class Foo::Bar` was invisible to the Ruby extractor, because a compound
  constant parses as `scope_resolution` and the extractor matched on node kind.
- Error classes and prop interfaces outvoted the type a file is actually about.
- Repository-wide shape rules let migrations speak for every Ruby file.
- The Ruby extractor recursed and overflowed the stack on a real file, aborting
  the process. It walks iteratively now.
