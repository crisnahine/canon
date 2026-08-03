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
