---
applyTo: "**"
---

# General Coding Instructions

These instructions apply to all files in this repository and supplement the guidelines in `.github/copilot-instructions.md`.

## Code Style

- Use consistent indentation: 4 spaces for most languages; 2 spaces for JSON, YAML, and HTML.
- End every file with a single newline character.
- Remove trailing whitespace on all lines.
- Limit line length to 120 characters where practical.

## Naming Conventions

- **Variables and functions**: `camelCase` (JavaScript/TypeScript/Go), `snake_case` (Python/Rust)
- **Classes and types**: `PascalCase`
- **Constants**: `UPPER_SNAKE_CASE`
- **Files**: `kebab-case` for most files; match language conventions for source files

## Import / Dependency Order

Order imports as: standard library → third-party → local modules. Separate each group with a blank line.

## Error Messages

Write error messages that are actionable: describe what went wrong, where, and how to fix it.
Example: `"Missing required environment variable DATABASE_URL — set it in .env or GitHub Secrets"`

## Comments

- Avoid restating what the code does — explain _why_ non-obvious decisions were made.
- Use `TODO(username):` and `FIXME(username):` prefixes for outstanding work items.
- Keep comments up to date when code changes.
