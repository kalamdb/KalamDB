# Skills docs (T123)

Updated canonical `../kalamdb-skills/skills/kalamdb/references/sql-syntax.md` and `cli.md` on 2026-09-07:

- LANGUAGE only with a body; project-backed procedures omit LANGUAGE/AS
- `ctx.db.query` / `execute`; REST success body is the return value
- `GRANT EXECUTE ... TO anonymous`; PUBLIC does not include anonymous
- `kalam functions build|status|revisions|rollback|logs|override`
- `kalam deploy --dry-run` is parse/generate/build/plan only

Generated skill mirrors under `kalamdb-skills/generated/` were not regenerated in this pass.
