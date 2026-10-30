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
- at least 1 approving review (`required_approving_review_count: 1`)
- a review from a code owner (`require_code_owner_review: true`, per
  `CODEOWNERS`)
- the most recent push to be re-approved (`require_last_push_approval: true`)
- stale approvals dismissed on new pushes (`dismiss_stale_reviews_on_push: true`)
- review threads resolved before merge (`required_review_thread_resolution: true`)

It also blocks deleting or force-pushing `main` (`deletion`,
`non_fast_forward` rules) — standard branch-protection baseline, independent
of the review requirement above.

## Self-approval bypass

`astorise` is the repository's only collaborator and the only owner in
`CODEOWNERS` (`* @astorise`). GitHub never lets a PR author approve their own
pull request, so with CODEOWNER review required and no bypass, no PR —
including ones Claude opens for audit fixes — could ever satisfy the review
requirement; it would stay open, unmergeable, forever.

`bypass_actors` now grants `astorise` (GitHub user id `6298458`, confirmed
from the live API rather than guessed) a `User`-type bypass with
`bypass_mode: "pull_request"` — bypassing the review-count/code-owner/
thread-resolution requirements only when merging via a pull request, never
a direct push to `main` (`deletion` and `non_fast_forward` still apply
unconditionally to everyone). A role-based bypass (`RepositoryRole`,
e.g. "Repository admin") was considered instead but dropped: GitHub's public
docs enumerate `RepositoryRole` as a valid `actor_type` without documenting
which numeric `actor_id` corresponds to which built-in role, and there is no
API access in this environment to query it directly — importing a guessed
value risks a silently wrong bypass grant. A `User`-type bypass sidesteps
that gap entirely, since this ID is directly confirmed, and is equivalent in
practice today: `astorise` is the only account that exists to bypass.

This does narrow automatically if a second collaborator is ever added:
unlike a role-based bypass, this one would not extend to them. Re-visit then
— either add their user id too, or switch to `RepositoryRole` once the
correct `actor_id` can be confirmed (e.g. by exporting an existing ruleset
configured with that bypass through the UI and reading back the id it
assigned).

### Confirmed live drift (2026-10-30 audit, unresolved)

Diffing this file against `astorise`'s 2026-10-30 export of the live
`main-branch-protection` ruleset (id `25885533`) found two differences from
what this file -- and the design reasoning above -- documents:

- **`require_code_owner_review` is `false` live**, not `true`. The CODEOWNERS
  review requirement described above is not actually enforced today.
- **`bypass_actors` is live `{"actor_id": 5, "actor_type": "RepositoryRole",
  "bypass_mode": "always"}`**, not the `User`/`pull_request` bypass this file
  specifies. This is broader in both scope (any account holding whatever role
  `actor_id: 5` names, not just `astorise`) and mode (`always` also bypasses
  `deletion`/`non_fast_forward`, which the design above specifically says
  should apply unconditionally to everyone) -- the exact `RepositoryRole`
  risk this file's own reasoning said it was avoiding by picking `User`
  instead. GitHub's REST API docs for `bypass_actors` (confirmed 2026-10-30)
  still don't enumerate which role each built-in `RepositoryRole` `actor_id`
  names, so which role `5` is remains unconfirmed.

Not resolved by this commit either direction (bringing the live ruleset back
in line with this file, or updating this file and the reasoning above to
match live reality) pending `astorise`'s decision on which is actually
intended.

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
