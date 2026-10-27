# Rules for agents in this repo

Everything you need to work here is in this repo. The full pipeline and rules (`PIPELINE.md`, `RULES.md`, `RUBRIC.md`, `roles/`) live in the owner's private pipeline repo; this is the short copy.

1. **Find the stage** in `PIPELINE.md`. The spec is `SPEC.md`, the plan `ROADMAP.md`, the code map `docs/code-map.md`.
2. **Work from an issue.** Take one that is not `in-progress` and not blocked; add `in-progress` before you start. Its body is your brief; an Easy issue has a "Qwen brief" section with the expected results and the code map snippet.
3. **Your own platform's issues first.** Mac, Windows and Linux all matter: say which you built on.
4. **Check that it builds**, then open the PR. CI and a review by another satellite with another model do the rest.
5. **Label the PR** `by:<machine>` and `model:<model>`, and write `Closes #N`.
6. **A failed job** gets a comment on the issue saying what happened, and the issue moves up one size (Easy -> Medium -> Hard).
7. **Use the project's words** from `GLOSSARY.md` and the decisions in `docs/adr/`.
8. **No private details** in code, commits or PR text: no names, e-mail addresses, home paths or machine names. Commit as the no-reply identity.

## This project

- **No C dependencies and no `unsafe`** (README). It is a library used by WavCraft and SoundCraft: keep the public API in README working, or bump the version.
- Plan: `ROADMAP.md`.
