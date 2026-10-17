# Project pages

Maintain shared roadmap and planning pages here as plain Markdown so everyone with repository access can read, review, and update them through Git.

## Pages

| Page | Purpose |
| --- | --- |
| [KalamDB 0.8 — Epics](kalamdb-0.8-epics.md) | PostgreSQL 18 compatibility roadmap, priorities, dependencies, design requirements, and release gates. |

## Editing workflow

1. Edit the relevant page in `docs/pages/` and submit the change through the normal repository pull request workflow.
2. Keep one file per page, using descriptive lowercase hyphenated filenames, and add new pages to this index.
3. Use relative links between repository documents and link to GitHub issues for implementation tasks, owners, and current progress.
4. When scope, priorities, dependencies, or acceptance criteria change, update this roadmap and the affected issue descriptions together. Date status snapshots; an unchecked roadmap item or a closed issue alone is not release evidence.
5. Treat repository pages as the source of truth. External Page copies are historical snapshots unless explicitly refreshed; there is no automatic synchronization.

These pages describe roadmap intent. Detailed implementation plans remain in [docs/plans](../plans/), specifications in [specs](../../specs/), and accepted architectural decisions in [docs/architecture/decisions](../architecture/decisions/).

## Sharing

Once changes are committed and pushed, collaborators can browse these files directly on GitHub or in their checkout. Local, uncommitted edits are not yet visible to other repository users.

