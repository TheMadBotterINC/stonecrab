# 0004: Use master as the default branch
## Status
Accepted by explicit maintainer instruction.
## Decision
The repository's local primary branch and remote default branch are `master`.
This overrides the `main` naming in the supplied coding standards §18 and the
hardware-job references in §19. Preserve the rest of the commit and review policy.
## Evidence
The remote default was changed to `master`. The redundant remote `main` contained
only the Apache license already present on `master` and was deleted after comparison.
No code commits were pushed as part of the branch cleanup.
