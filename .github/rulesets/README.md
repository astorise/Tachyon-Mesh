# Repository rulesets (TACH-12)

`main-branch-protection.json` and `main.json` are this repo's two active
GitHub rulesets, kept here so the live configuration has a versioned record
to diff against. Claude Code has no API access to repository rulesets in
this environment (confirmed again on 2026-10-30: no tool in the connected
GitHub MCP server's surface covers the `rulesets` endpoints), so neither
file can be applied, nor the live configuration read back, automatically --
`astorise` exported both live rulesets by hand (Settings → Rules →
Rulesets → open each ruleset → "..." → Export as JSON) for reconciliation.

## Import

Settings → Rules → Rulesets → New ruleset → Import a ruleset → select the
file. Review the imported rules, then set **Enforcement status** to
**Active**.

(Equivalent via the API: `POST /repos/{owner}/{repo}/rulesets` with the
file's contents as the body.)

## `main-branch-protection.json` -- what it does

Targets the default branch (`main`) and requires, before a pull request can
merge:
- at least 1 approving review (`required_approving_review_count: 1`) --
  from any collaborator, not specifically a `CODEOWNERS` entry
  (`require_code_owner_review: false`)
- the most recent push to be re-approved (`require_last_push_approval: true`)
- stale approvals dismissed on new pushes (`dismiss_stale_reviews_on_push: true`)
- review threads resolved before merge (`required_review_thread_resolution: true`)

It also blocks deleting or force-pushing `main` (`deletion`,
`non_fast_forward` rules) for everyone except the role bypass below.

## Role bypass

`bypass_actors` grants a built-in `RepositoryRole` (`actor_id: 5`) a bypass
with `bypass_mode: "always"` -- unlike `bypass_mode: "pull_request"`, this
exempts that role from every rule in this ruleset unconditionally,
`deletion`/`non_fast_forward` on a direct push included, not only the
PR-review rules above. GitHub's REST API docs for `bypass_actors` don't
enumerate which built-in role each numeric `RepositoryRole` `actor_id`
names (confirmed 2026-10-30), so which role `5` actually is remains
unconfirmed -- on this single-collaborator repo, where `astorise` is both
the only account and (almost certainly) the only holder of whichever role
this is, this is operationally equivalent to an explicit per-user bypass.

This does narrow automatically if a second collaborator holding the same
role is ever added: unlike a per-user bypass, this one would extend to
them too, without anyone having granted it to them explicitly. Re-visit
then if that's not intended.

### 2026-10-30 audit: reconciled with live configuration

An earlier version of this file specified `require_code_owner_review: true`
and a narrower `User`-type bypass for `astorise` specifically
(`bypass_mode: "pull_request"`), reasoning that a `RepositoryRole` bypass
was too risky to configure blind without confirming which numeric
`actor_id` maps to which built-in role. `astorise`'s live export of this
ruleset (id `25885533`) showed the actual live configuration had since
diverged to exactly the `require_code_owner_review: false` /
`RepositoryRole`(5)/`always` bypass this file now specifies above --
confirmed as the intended configuration, so this file and the reasoning
above were updated to match live reality rather than the reverse.

## `main.json` -- what it does

A second, independent ruleset also targeting `main` (by literal branch name,
`refs/heads/main`, rather than this repo's other ruleset's `~DEFAULT_BRANCH`)
that existed live with no versioned record at all until this commit --
reconstructed verbatim from `astorise`'s 2026-10-30 export (id `23935661`).
Requires, before a pull request can merge, that `quality`, `cuda-quality`,
`security-audit`, and `build-guests` (CI's own job names, matched by
`integration_id` 15368 -- GitHub Actions) all report success on the head
commit (`required_status_checks`, `strict_required_status_checks_policy:
true`) -- this is what TACH-17/TACH-18's "a tag can never ship from a commit
whose CI didn't pass" guarantee actually rests on at the branch-protection
layer. Also blocks deleting or force-pushing `main`, same as
`main-branch-protection.json`.

Its own `pull_request` rule requires 0 approving reviews
(`required_approving_review_count: 0`). GitHub's docs on rule layering
confirm multiple rulesets targeting the same branch are aggregated, and
"if the same rule is defined in different ways across the aggregated
rulesets... the most restrictive version of the rule applies" -- so the
two rulesets together still require the 1 approval
`main-branch-protection.json` asks for; this one simply doesn't raise
that bar on its own.
