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

## Known caveat: this will block every merge as currently written

`bypass_actors` is intentionally left empty. `astorise` is the repository's
only collaborator and the only owner in `CODEOWNERS` (`* @astorise`).
GitHub never lets a PR author approve their own pull request, so with this
ruleset **active and no bypass**, no PR — including ones Claude opens for
future audit fixes — can ever satisfy the review requirement. The PR stays
open, unmergeable, forever.

Left out deliberately: a `bypass_actors` entry for the repository-admin role
needs a numeric `actor_id` for that built-in role, and that ID isn't
confirmed from GitHub's public docs — importing a guessed value risks a
silently wrong bypass grant instead of the intended one. Adding it by hand
is one step and safer:

Settings → Rules → the imported ruleset → **Bypass list** → **Add bypass** →
role **Repository admin**, bypass mode **Always**.

Alternatives instead of a bypass: add a second human or bot reviewer who can
satisfy the review requirement, or keep enforcement set to **Evaluate**
(dry-run, reports what would be blocked without blocking anything) until
one of the above is in place.
