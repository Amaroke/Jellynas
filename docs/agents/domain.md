# Domain Docs

This repo has a single context.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root, the glossary.
- **`docs/adr/`**: the ADRs that touch the area you are about to work in.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR 0009 (ratio farms C411), but worth reopening because…_
