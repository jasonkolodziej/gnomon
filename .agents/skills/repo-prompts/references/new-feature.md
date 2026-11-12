---
name: new-feature
description: "Scaffold a new feature: create the implementation file, a corresponding test file, and update the README if needed."
disable-model-invocation: true
---
# New Feature Scaffold

Your task is to implement a new feature as described below. Follow all guidelines in `.github/copilot-instructions.md`.

## Instructions

1. **Understand the requirement** — ask clarifying questions if the description is ambiguous before writing any code.
2. **Create the implementation** in the appropriate directory, following existing code style and conventions.
3. **Create a test file** alongside or in the `tests/` directory, covering at least the happy path and one error/edge case.
4. **Update `README.md`** if the feature changes public-facing behavior, APIs, or configuration.
5. **Draft a conventional-commit message** summarising the change (`feat(scope): description`).

## Feature Description

${input:featureDescription:Describe the feature to implement (what it should do and why)}

## Relevant Context

- Existing files to consider: ${input:relatedFiles:List any related files or leave blank}
- Additional constraints: ${input:constraints:Any constraints or non-functional requirements}
