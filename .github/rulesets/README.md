# Repository rulesets (TACH-12)

`main-branch-protection.json` is a GitHub ruleset ready to import. Claude Code
has no API access to repository rulesets in this environment, so it cannot be
applied automatically — it is generated here for `astorise` to import by hand.

## Import

Settings → Rules → Rulesets → New ruleset → Import a ruleset → select
`main-branch-protection.json`. Review the imported rules, then set
**Enforcement status** to **Active**.

(Equivalent via the API: `POST /repos/{owner}/{repo}/rulesets` with this
file's contents as the body.)

## What it does

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
