# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

A live false refusal, and the five rules that answer why it happened: a base
class is not how most frameworks say what a file is.

### Fixed

- **A Django view was refused for inheriting from its own mixin.** Python's
  base list is positional and its frameworks order it mixins-first, so reading
  the first entry made `class OrderView(LoginRequiredMixin, ListView)` a
  subclass of `LoginRequiredMixin`. Measured on a Django codebase, the first
  base ends in `Mixin` in 337 of 754 declarations. The last positional base is
  the type the class is; everything before it is composition and lands in
  `shape.mixin` instead. Go's embedded fields are read the same way, having no
  order at all. A base read from a language that allows several is advisory
  regardless, for Rust and Python both. The ordering is a convention rather
  than a fact, and refusing on it is the check being wrong about a correct file.

- **A parameterised base agreed with nothing.** `ActiveRecord::Migration[7.2]`
  and `ActiveRecord::Migration[6.1]` are one base spelled per Rails version, and
  compared as written a migrations directory held six of them and agreed on
  none: the largest single spelling was 445 files of 1,518. `db/migrate` now
  derives `ActiveRecord::Migration` at 1519/1519. A call expression is still
  kept whole, because `Struct.new(:street, :city)` builds a different anonymous
  type per argument list, including when that argument list holds a `[`. That
  is `class Point(namedtuple('Point', ['x', 'y']))`, and it used to yield the
  base `namedtuple('Point',`.

- **A qualified base lost its qualifier.** A statement is an instruction, so a
  directory whose types inherit `models.Model` is told `models.Model`, which is
  the line to write. Rust trait impls now keep theirs too: `serde::Serialize`,
  not `Serialize`, which is only correct in a file that already imported it.

- **A Ruby call with a receiver was recorded twice.** Ruby keeps `receiver` and
  `method` as sibling fields of one node, so the query written for a
  receiverless call matched a receiver call as well and `Payment.charge(1)`
  arrived as two facts: one correct, one claiming a receiverless `charge` the
  source never wrote. Every rule reading a receiverless call saw it, and a
  Rails repository derived `find`, `create`, `new` and `gsub` as macros whole
  directories agreed on. No other language keeps the two in one node, and none
  had the problem.

- **Two rules stated one fact twice.** A Rust type's superclass *is* its first
  trait impl, so a directory derived both "Types here inherit from `Loggable`"
  and "Types here implement `Loggable`". Two lines of the injected budget, and
  two claims the checker could not collapse. The base rule yields to the
  contract rule there. Separately, the family rule and the base rule are
  withheld from each other only at the same scope, and scopes nest: a Rails API
  derives the family rule over `app/controllers` and the exact base over
  `app/controllers/api/v1`, so one wrong base produced two lines saying
  different things. The narrower rule now wins.

- **The annotation a directory shares lost to the one that sorted last.** A
  NestJS controller carries `@Controller` once and `@Get`, `@Post` and
  `@Delete` once each, so every within-file count ties at one and the tie
  resolved to whichever name sorted last. Both the annotation and the macro
  family now count a name once per file that carries it at all.

- **The commit-time walk gave up on a workspace of checkouts.** `head_sha` and
  `tracked_files` both answer for an `api/ client/ wordpress/` layout through
  its children; this returned nothing, so every file in every checkout fell
  back to its mtime, which is one distinct value across a fresh clone. It
  reads the children now, and reads NUL-separated output, because a path
  containing a newline is legal and splitting on newlines recorded two paths
  that do not exist.

- **`reconcile` paid for an unbounded history walk every turn.** The only thing
  it does with a commit time is order a directory's files before truncating
  them to a shortlist. It reads the recent commits now; anything older falls
  back to its mtime. Deriving still reads the whole log, where a commit time is
  a weight on every vote.

- **A rule's stored grade could disagree with the one recomputed on write.**
  Four derive sites handed `enforcement_for` the rule family rather than the
  full id, and its guards read the rest of that id, so a Python base rule at
  total agreement was stored `Blocking` and recomputed `Advisory`.

- The annotation and macro checks printed a bare "(95/102)". Every other check
  names the file set behind the number.

### Added

- **Five rule kinds, for what a framework asks for rather than what a class
  inherits.** `shape.annotation` reads the decorator or attribute a file
  carries; `shape.macros` a call with no receiver; `shape.mixin` a module or
  trait a type composes in; `shape.contract` an `implements` clause or a Rust
  trait impl; `shape.family` the suffix several namespaced bases share, stated
  only where no single base won.

  Measured, in conventions derived: the Rails API 146 to 262, `pixelfed` 122
  to 142, `nest` 107 to 136, `wagtail` 74 to 101, `nuxt-ui` 34 to 46,
  `starship` 7 to 10. The answers matter more than the counts. `app/workers` derives `Sidekiq::Worker`
  at 485/490, where 485 of those files declare no base at all and every
  class-shaped rule was silent about them. Pixelfed's `app/Jobs` derives
  `ShouldQueue` at 119/119 from an `implements` clause the extractor had
  recorded for its whole life and nothing had ever read. `app/controllers`
  derives `*BaseController` at 95/102, where the largest single spelling is 53
  and the exact rule finds no winner.

  All five are advisory. Each is a fact about what a directory mostly does, and
  each has a legitimate exception: the one plain helper class beside the
  decorated ones, the one worker that composes something else.

- **A directory is counted over its own files as well as over its subtree.** A
  rule was only ever derived by path prefix, so a directory holding a
  subdirectory of another kind counted both kinds in one vote. `app/models` in
  the Rails API holds 128 models, 123 of which inherit `ApplicationRecord`,
  beside 36 concerns, which are modules and inherit nothing: 123 of 164 is
  under the bar a sample that size has to clear, and all 128 models derived no
  rule about their base at all. `canon explain app/models/` now answers
  `` Types here inherit from `ApplicationRecord` `` at 123/128 over
  `app/models/*.rb`, and `` Files here use `belongs_to` `` at 115/128 beside
  it, with the concerns keeping every rule they had.

  The new `Scope::DirChildrenExt` reaches one directory and nothing below it,
  which makes it the narrowest scope canon has and puts it first in the
  injected block. It is derived only where the subtree scope had no answer of
  that kind for that directory, so nothing that was stated before is restated,
  displaced or regraded — the subtree scope is the better answer whenever it
  exists, because it is the one a folder created after indexing inherits from.
  A rolled-up rule counts as such an answer, which is why the two passes run in
  that order.

### Changed

- **The grouping depth cap, 4 to 8.** Rules are derived at every ancestor
  directory up to a cap, and at four it was binding rather than generous: a
  snapshot's scope-depth histogram stopped dead at 4 on every repository
  measured, while 25% of the Rails API's Ruby files and 52% of the React
  client's TypeScript files lived deeper and could never have a rule of their
  own. The sharpest case is the layout canon documents as a feature. A
  workspace holding several checkouts prefixes every path with the checkout
  name, so `api/app/services/billing` is already at the cap: opening
  `empire-flippers/` derived 152 rules where `api` and `client` opened
  separately derived 285 between them. It now derives 450.

  Eight by measurement. Against 4, caps of 6, 8 and 10 were measured on eight
  real repositories: 6 recovers most of the loss, 8 recovers effectively all of
  it, and 10 adds two rules on one repository and three on the workspace for
  the same derivation cost. Cost is bounded from the other side by `min_files`,
  which derives nothing from a group too small however deep it sits.

- `SNAPSHOT_VERSION` 10 to 15. The snapshot is a cache keyed on the commit, its
  age and the settings, so a new binary at an unchanged commit goes on serving
  the old binary's conventions, including, from a version 10 snapshot, the
  first-positional-base reading that refused a Django view.

## [0.5.0] — 2026-08-04

Six issues, and the two that changed the most were not what their reports said.

### Fixed

- **Derivation was not deterministic: the same tree gave 50 to 54 conventions
  (#21).** Two causes, and the issue named only one. `roll_up_agreeing_siblings`
  read `HashMap` order, so a rolled-up rule was named after a different child
  each run and carried a different twelve evidence files; votes and holders are
  sorted now, and `widest` breaks a tie on lowest id. That alone did not fix the
  count. The swing came from `recency_weight`, a continuous function of the wall
  clock feeding a comparison against a fixed bar: two rebuilds ninety seconds
  apart put a rule sitting on its threshold on opposite sides of it. Age is read
  in whole days now, so the answer is the same all day. Measured: 146/146/146/146
  on a 9,557-file repository, byte-identical conventions across six rebuilds.

  Rollup ids can change on upgrade because of this. A tie on sample size used to
  resolve by iteration order and now resolves to the lowest id, so a rule
  previously called `tests.colocation.app.services.google.rb.rollup` may come
  back under a sibling's name. If you suppress a `.rollup` id in `.canon.toml`,
  check it still matches — the suppression is resolved against live config, so a
  renamed id silently stops being suppressed.

- **`confidence_floor` was inert below 0.8 and multiplied refusals fourfold when
  raised (#20).** Below 0.8 nothing survived to admit, because
  `Confidence::derive_counted` had already refused it; the validated range is
  0.8 to 1.0 now and a lower value is a load error rather than a setting that
  does nothing. Above the default it did the opposite of its name: applied
  inside the majority vote it killed the wide rule first, and every narrow rule
  the wide one would have absorbed survived — narrow rules at total agreement
  being exactly what grades `Blocking`. At 1.0 that was 179 conventions against
  138, and 113 rules that may refuse a write against 26. The floor is a filter
  over the finished set now, after roll-up and collapsing: 146 / 72 / 49 at
  0.8 / 0.95 / 1.0, and the refusal count never rises.

- **One vendored filename could pin a `Blocking` naming rule for a whole
  directory (#19).** `is_discriminating` accepted any `_` or `-` as a word
  boundary, so `jquery-3.4.1.min.js` — root `jquery-3` — witnessed `kebab-case`
  for a directory whose four other files were single words, at total agreement.
  A separator now witnesses only with a non-numeric segment on both sides.
  `create_v2`, `user-2fa` and `oauth2-client` still witness. Excluding `*.min.js`
  or a `vendor/` path list would have fixed the instance and left the class.

- **A naming rule refused in every sampled subdirectory but not at its own scope
  root (#18).** The guard asked only whether the file sat *under* a counted
  directory, so where every sample lived in a subdirectory the scope root
  answered neither way. Scopes that name a directory accept the ancestor
  direction now, fenced by their own prefix; scopes that do not — `Scope::Ext`,
  `Scope::Repo` — deliberately still refuse it, or a `**/*.md` rule counted in
  `docs/` would start refusing a root file. The empty-string sample root, which
  records a file counted at the repository root, no longer licenses every
  directory in the tree.

- **One defect was reported once per nesting level, and the six-line cap then
  dropped a distinct fact (#17).** A file breaking three rules in a three-level
  directory emitted eight violations carrying four facts, and the fourth — this
  file has no test — was the line that fell off the end. Violations are keyed on
  the claim now, the narrowest scope wins, and the key is the defect rather than
  the expected value, so two levels that disagree still produce one line instead
  of two contradictory ones.

### Added

- **A vocabulary for the languages the first one could not describe (#16).**
  "Types here inherit from `X`" and "types here expose exactly one public method"
  describe a service-object codebase exactly and a React component, a view
  template and a WordPress plugin file not at all, so those trees derived almost
  nothing however many files they held. Three families, all advisory, none able
  to refuse:

  | Rule | Reads | Example |
  |---|---|---|
  | `format` | the segment between the name and the extension | Files here are named `*.html.erb` |
  | `shape.export` | whether the module exports a default | Files here export a default |
  | `shape.namespace` | the namespace the file declares | Files here declare namespace `App\Services\Billing` |

  Measured: a 9,557-file Rails repository went from one ERB rule to nine, a
  3,189-file React repository gained 39 export-style rules, and a 698-file
  WordPress theme derived its first structural rule at all.

  `shape.namespace` is the one family derived per exact directory rather than at
  every ancestor. PSR-4 makes a subdirectory's namespace differ from its
  parent's by definition, so an ancestor rule is not merely less specific for a
  file below it — it is wrong. It is excluded from roll-up and from collapsing
  for the same reason.

  `FileFacts` gained `default_export` and `namespace`, and `Provider` gained
  `default_exports` so a language without the concept abstains rather than
  agreeing unanimously about a thing it has no word for.

### Changed

- `SNAPSHOT_VERSION` 9 to 10. The snapshot is a cache keyed on the commit, its
  age and the settings, so a new binary at an unchanged commit goes on serving
  the old binary's conventions — and a version 9 snapshot holds the vendored
  `jquery-3` rule above, graded `Blocking`, which would go on refusing writes
  after the release that stopped deriving it.

- `confidence_floor` no longer accepts 0.5 to 0.79. Those values passed
  validation, were printed by `canon check`, and changed no rule.

  **If your `.canon.toml` sets one, raise it to 0.8 before upgrading.** A file
  the loader rejects is not a partial failure: every setting in it reverts to
  the default and enforcement is switched off, so suppressions stop applying
  and nothing refuses a write. That fallback is deliberate, because the setting
  that can block a write has to fail toward permissive, but until now it also
  happened in silence. Session start says it out loud from this release.

- A raised `confidence_floor` can now leave a directory with no rule where the
  default would have given it one. The floor is read over the finished set, so
  a wide rule at 0.92 that had already absorbed a narrow one at 1.00 is removed
  without the narrow one coming back. That is the cost of the ordering: read
  earlier, the floor removes the wide rule before it can absorb anything and
  the narrow copies all survive, which is the fourfold refusal increase this
  release fixes. Monotonicity was the property worth keeping.

## [0.4.2] — 2026-08-03

### Fixed

- **`canon explain <path>` matched nothing on Windows.** `normalise_query`,
  added in 0.4.1, resolved the argument through `Path` and returned it with the
  platform separator, while scopes and evidence are stored with forward slashes
  because that is what `git ls-files` reports. So `app\services` was compared
  against `app/services` and `canon explain app/services` answered "no
  conventions match" — the audit surface every refusal points at, on a
  supported platform.

  `relative_to` has always converted separators; the new function was written
  beside it and left the step out. Both go through one helper now, and the
  regression test asserts the absence of a platform separator rather than a
  literal string, so it fails on Windows instead of passing everywhere.

  Caught by CI on the 0.4.1 tag, which is what the Windows job is for.

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


### Fixed — what a second review round found in the first round's fixes

Told the first round's ground was covered, a second round found seven more
criticals. Four of them were in the fix for the first round's headline defect,
which is the argument for the second round.

- **The scope-versus-sample guard switched itself off** as soon as a rule's
  sample spanned two top-level directories — markdown in `docs/` and
  `website/`, which is every project with a docs site. The rule it was written
  to stop then refused `.github/PULL_REQUEST_TEMPLATE.md` again. Conventions
  now carry the complete set of directories their sample came from, rather than
  it being inferred from twelve capped evidence paths.
- **One branch skipped that guard entirely.** `blocking_violations` falls
  through to a path-only check whenever the resulting file cannot be known — a
  `NotebookEdit`, or an `Edit` whose `old_string` no longer matches — and that
  branch refused what an identical `Write` allowed. The same
  "depends which tool the model reached for" defect, one branch over from where
  it was fixed.
- **An acronym was still refused everywhere except `PascalCase`.** Relaxing
  `Style::Pascal` fixed `SEO.tsx` in a components directory and left
  `docs/FAQ.md` and `docs/API.md` refused by a `kebab-case` rule. A
  separator-free all-uppercase name is how every project spells an acronym,
  whatever style it otherwise holds.
- **The nested-type filter discarded the module holding the real subject.** A
  root-level marker — `pub struct Sealed;`, a unit error type — was enough to
  drop a public module with the actual type in it, and the file then resolved
  to something with no surface at all. A root-level type only wins when it has
  methods of its own.
- **Three unguarded reads, each of which hangs forever on a FIFO**: `verify`
  reading the written file, `reconcile` reading everything touched this turn,
  and `load_file` reading `.canon.toml` — the last of which runs before every
  hook, including the one that would have reported it. A character device was
  measured streaming to 1.8 GB before the process was killed. This is the only
  failure a fail-open harness cannot report, because there is no output to
  inspect. One shared `read_indexable` now guards all of them.
- `canon explain` normalises its path argument, so `./app/services` and an
  absolute path answer the same as the relative one, and decides
  file-versus-directory from the snapshot rather than from punctuation — so
  `.github` and `api.v2` stop reporting "no conventions match".

### Verified — the regression tests were themselves tested

Every one of the 33 refusal assertions was run against the binary from before
its fix. Twelve fail on the immediately preceding commit; three more need the
commit before that. Two assertions in the first batch turned out to pass with
or without the fix they claimed to pin — one was saved by a stem name-match
that short-circuits the code path it tested, the other by a fixture that never
derived a rule of the relevant kind. Both were rebuilt until they failed on the
old binary.

The corpus is seventeen repositories now: 15,974 tracked files replayed and
5,946 new files written into real directories, none refused. `SNAPSHOT_VERSION`
is 9.


### Fixed — a third review round, and six defects older than any of it

Round three found twelve more criticals. Four were inside round two's fixes;
six predate all three rounds and were reached by digging past the diff into the
extractors.

Inside the previous round's work:

- **A trait impl was counted as a type's own surface.** Rust forbids `pub fn`
  inside `impl Trait for Type`, so every trait-impl method is filed as private —
  and one `impl Display for Sealed` was enough to make a marker type look like
  it had surface, discard the module holding the real subject, and refuse the
  file for exposing nothing. Every unit error type in Rust has such an impl,
  because `std::error::Error` requires `Display` and `Display` has no derive.
  The committed test stayed green because its marker carried no impl.
- **The same miscount decided which type a file was about.** A companion error
  type gets one private method from its `Display` impl, tying it with a
  one-public-method subject, and `max_by_key` returns the *last* maximum — so
  the subject was whichever of the two the author wrote second.
- **A bare acronym was exempt when checking and counted when deriving.** One
  `docs/FAQ.md` deleted every markdown naming rule in the repository. Deriving
  and checking have to agree on which names the style system reaches, and the
  previous round changed only one of them.
- **The sample-coverage guard used a top-level directory and only applied to
  `Scope::Ext`.** A repository whose source lives under `src/` recorded
  `["src"]`, so a rule counted in `src/components/` still refused
  `src/hooks/`, `src/pages/` and `src/utils/`, and a `src/**/*.tsx` rule got no
  directory check at all. The whole directory is recorded now, capped, and the
  check applies to every scope.

Older than all three rounds:

- **Ruby's `def self.call` was invisible.** It parses as `singleton_method`, not
  `method`, so a class-method service object — the `ChargeCard.call(...)` style
  canon's own README uses as its example — had no surface at all and was
  refused for exposing none.
- **A Ruby base written `::ApplicationService` was refused** by a rule naming
  `ApplicationService`. They are the same constant; the `::` only forces
  top-level lookup, and inside a namespaced module it is the only way to reach
  one.
- **Python read only a bare identifier as a base**, so `class X(base.Service)`
  and `class X(Service[Order])` both came out with no base and were then refused
  for having none.
- **Rust recorded whichever trait `impl` appeared first as the base**, making a
  refusal depend on the order the author wrote the blocks. Every implemented
  trait is kept now, and the check accepts any of them. A Rust trait is also no
  longer enforceable as a base at all: it is a contract a type opts into, not a
  structural parent, so a type that does not implement what its neighbours do is
  ordinary Rust.
- **The subject of every `PascalCase`- or `camelCase`-named file was resolved by
  surface rather than by name**, because `to_snake(type)` was compared against
  the stem as written — which cannot match for any JavaScript, TypeScript or PHP
  file. Both sides are normalised now.
- **`shape.public-arity` refused an extra method**, including the `up`/`down`
  pair Rails requires for an irreversible migration, a Go type implementing
  `fmt.Stringer`, and a Ruby object defining `to_s`. It advises on a larger
  surface now and refuses only a smaller one.
- **`Snapshot::load`, `take_touched` and the log file were unguarded reads**,
  each hanging forever on a FIFO. `Snapshot::load` is the first read every hook
  performs.

### Verified

- Every one of the eight open issues has a check in a harness that runs against
  any build: 23 assertions, all passing, each failing on the build before this.
- Every critical from all three review rounds has one too: 50 assertions, each
  run against the binary from before its fix to confirm it fails there. Three
  did not, when first written, and were rebuilt until they did.
- 15,974 tracked files from the seventeen repositories replayed through the
  write path: 11,700 given conventions, none refused, none errored.
- 5,946 *new* files written into those same directories — one file's content at
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
