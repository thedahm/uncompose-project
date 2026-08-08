# TASK

Merge the following branches into `{{SPEC_BRANCH}}`, each via a pull request so the work leaves a paper trail on GitHub:

{{BRANCHES}}

For each branch, one at a time:

1. Push it: `git push -u origin <branch>`
2. Open a PR: `gh pr create --base {{SPEC_BRANCH}} --head <branch> --title "<issue title>" --body "<one-paragraph summary of what the branch does, referencing its issue like #42>"`
3. Merge it with a merge commit: `gh pr merge <branch> --merge`
4. If GitHub reports the PR is not mergeable because of conflicts, resolve them locally first:
   - `git checkout <branch>` then `git merge {{SPEC_BRANCH}} --no-edit`
   - Resolve conflicts intelligently by reading both sides and choosing the correct resolution
   - Run `npm run typecheck` and `npm run test`; fix failures before continuing
   - `git push`, switch back to `{{SPEC_BRANCH}}`, and merge the PR as in step 3
5. After the PR merges, `git checkout {{SPEC_BRANCH}} && git pull` so the next branch merges against the latest state

After all branches are merged, run `npm run typecheck` and `npm run test` on `{{SPEC_BRANCH}}` one final time. If something fails, fix it, commit, and push.

# CLOSE ISSUES

PRs merging into `{{SPEC_BRANCH}}` do not auto-close issues (GitHub only does that against the default branch), so close each merged branch's issue explicitly:

`gh issue close <ID> --comment "Completed by Sandcastle in <PR URL>"`

Here are all the issues:

{{ISSUES}}

Once you've merged everything you can, output <promise>COMPLETE</promise>.
