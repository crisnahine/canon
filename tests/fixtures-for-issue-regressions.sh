#!/usr/bin/env bash
# Scratch repositories that exercise each open issue.
set -eu
M="$1"; rm -rf "$M"; mkdir -p "$M"
mk() { mkdir -p "$(dirname "$2")"; printf '%s' "$3" > "$2"; }
commit() { (cd "$1" && git init -q . && git add -A && git -c user.email=a@b -c user.name=t commit -qm x >/dev/null); }

# --- services: a directory with three Blocking shape rules and a naming rule ---
R="$M/services"
for n in charge_card refund_payment settle_batch send_receipt void_invoice apply_credit; do
  C=$(python3 -c "print(''.join(w.capitalize() for w in '$n'.split('_')))")
  mk "$R" "$R/app/services/$n.py" "from app.services.base import BaseService


class $C(BaseService):
    def execute(self, payload):
        return self._run(payload)

    def _run(self, payload):
        return payload
"
done
commit "$R"

# --- extonly: the only rule for .rake is repository-wide (Scope::Ext) ---
R="$M/extonly"
for n in one_backfill two_backfill three_backfill four_backfill five_backfill six_backfill; do
  mk "$R" "$R/lib/tasks/$n.rake" "task :$n do\nend\n"
done
mk "$R" "$R/README.md" "x\n"
commit "$R"

# --- denominator: .txt rule whose sample excludes LICENSE and fixtures ---
R="$M/denominator"
for n in alpha_note beta_note gamma_note delta_note epsilon_note zeta_note eta_note theta_note; do
  mk "$R" "$R/docs/$n.txt" "note\n"
done
mk "$R" "$R/LICENSE.txt" "license\n"
mk "$R" "$R/spec/fixtures/ips-v4.txt" "1.2.3.4\n"
mk "$R" "$R/spec/fixtures/ips-v6.txt" "::1\n"
commit "$R"

# --- families: derives colocation, import and test-suffix rules ---
R="$M/families"
for n in charge_card refund_payment settle_batch send_receipt void_invoice apply_credit; do
  C=$(python3 -c "print(''.join(w.capitalize() for w in '$n'.split('_')))")
  mk "$R" "$R/app/services/$n.rb" "require 'application_service'

class $C < ApplicationService
  def call
    run
  end

  private

  def run; end
end
"
  mk "$R" "$R/spec/services/${n}_spec.rb" "require 'rails_helper'

RSpec.describe $C do
end
"
done
commit "$R"

# --- nested: same naming statement derived at an ancestor and a child scope ---
R="$M/nested"
for n in alpha_note beta_note gamma_note delta_note epsilon_note; do
  mk "$R" "$R/api/$n.txt" "x\n"
done
for n in kappa_note lambda_note mu_note nu_note xi_note; do
  mk "$R" "$R/web/$n.txt" "x\n"
done
commit "$R"

# --- mixed: several extensions and directories, for `explain` filtering ---
R="$M/mixed"
for n in alpha_note beta_note gamma_note delta_note epsilon_note; do
  mk "$R" "$R/data/$n.csv" "a,b\n"
done
for n in one_backfill two_backfill three_backfill four_backfill five_backfill; do
  mk "$R" "$R/lib/tasks/$n.rake" "task :x do\nend\n"
done
for n in charge_card refund_payment settle_batch send_receipt void_invoice; do
  C=$(python3 -c "print(''.join(w.capitalize() for w in '$n'.split('_')))")
  mk "$R" "$R/app/models/$n.rb" "class $C < ApplicationRecord\nend\n"
done
commit "$R"

# --- quiet: six Python files that derive no Tier 1 conventions ---
R="$M/quiet"
for n in alpha beta gamma delta epsilon zeta; do
  mk "$R" "$R/pkg/$n.py" "VALUE = 1
OTHER = 2
"
done
commit "$R"

# --- vendored: four ordinary scripts and one version-numbered library (#19) ---
R="$M/vendored"
for n in main plugins helpers widgets; do
  mk "$R" "$R/js/$n.js" "var $n = 1;
"
done
mk "$R" "$R/js/jquery-3.4.1.min.js" "var j=1;
"
commit "$R"

# --- views: an ERB tree whose format segment is the only rule it has (#16) ---
R="$M/views"
for n in index show edit confirm receipt summary; do
  mk "$R" "$R/app/views/orders/$n.html.erb" "<h1>x</h1>
"
done
commit "$R"

# --- nested: shape rules at two levels over one defect (#17) ---
R="$M/nested2"
for d in billing/invoices billing/payments; do
  for n in One Two Three Four Five Six Seven; do
    l=$(printf '%s' "$n" | tr 'A-Z' 'a-z')
    mk "$R" "$R/app/services/$d/${l}_service.rb" "class ${n}Service < RightBase
  def run; end
end
"
  done
done
for n in Odd Even Extra Spare; do
  l=$(printf '%s' "$n" | tr 'A-Z' 'a-z')
  mk "$R" "$R/app/services/billing/${l}_service.rb" "class ${n}Service < OtherBase
  def run; end
end
"
done
commit "$R"

# --- scoperoot: every sampled file in a subdirectory, none at the root (#18) ---
R="$M/scoperoot"
for d in one two; do
  for n in Alpha Beta Gamma Delta Epsilon Zeta; do
    mk "$R" "$R/src/components/group/sub$d/$n$d.tsx" "export const X = 1;
"
  done
done
# A differently-styled sibling tree, so no rule forms wide enough to absorb the
# one under test and the surviving scope really is `src/components`.
for n in first-page second-page third-page fourth-page fifth-page sixth-page; do
  mk "$R" "$R/src/pages/$n.tsx" "export const X = 1;
"
done
commit "$R"

# --- floorsplit: children unanimous, parent split by one dissenter (#20) ---
R="$M/floorsplit"
for d in alpha beta; do
  for n in One Two Three Four Five Six; do
    l=$(printf '%s' "$n" | tr 'A-Z' 'a-z')
    mk "$R" "$R/app/services/$d/${l}_${d}.rb" "class ${n}${d} < ApplicationService
  def call; end
end
"
  done
done
mk "$R" "$R/app/services/odd_one.rb" "class OddOne < SomethingElse
  def call; end
end
"
commit "$R"

# --- rolluptie: two equal-sized children, so the rollup has a tie to break (#21) ---
R="$M/rolluptie"
for n in taskList taskStyles parseSheet miscIndex feedbackSchema questionSchema sellerAvailability valuationIndex; do
  mk "$R" "$R/src/components/TaskList/$n.ts" "export const x = 1;
"
done
for n in buyOrSell processEmailEvent emailChecker onboardingSchema stepInitial stepPassword stepSuccess stepNew; do
  mk "$R" "$R/src/components/UniversalOnboarding/$n.ts" "export const x = 1;
"
done
for n in Legacy OtherThing; do
  mk "$R" "$R/src/components/$n.ts" "export const x = 1;
"
done
commit "$R"

