# GitHub Copilot Instructions

This file provides repository-wide custom instructions for GitHub Copilot across all agents (VS Code inline, Copilot Chat, CLI, and cloud agents).

## Repository Purpose

This is a **GitHub repository template** designed to provide a consistent, best-practice starting point for new projects. It includes Copilot configuration, CI/CD workflows, VS Code workspace settings, and reusable agent skills/prompts.

## Coding Guidelines

- Prefer clear, self-documenting code over excessive inline comments.
- Follow existing file and directory naming conventions in the repository.
- Write small, focused functions with a single responsibility.
- Add error handling for all I/O and network operations.
- Keep dependencies minimal — prefer standard library solutions when possible.

## Commit & Pull Request Guidelines

- Use [Conventional Commits](https://www.conventionalcommits.org/) format: `type(scope): description`
  - Common types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `ci`
- Keep commits atomic — one logical change per commit.
- Reference issue numbers in commit messages and PR descriptions when applicable.
- Every PR should include a description of _what_ changed and _why_.

## Testing Guidelines

- Write tests alongside feature code (co-located or in a `tests/` directory).
- Aim for meaningful test coverage of public APIs and critical paths.
- Tests should be deterministic and independent of each other.
- Use descriptive test names that explain the expected behavior.

## Documentation Guidelines

- Update `README.md` when adding new features or changing configuration.
- Document public APIs, configuration options, and non-obvious decisions.
- Place architectural decisions in `.github/plans/` as ADR (Architecture Decision Record) files.

## Security Guidelines

- Never commit secrets, credentials, or API keys — use GitHub Secrets or environment variables.
- Validate and sanitize all external inputs.
- Keep dependencies up to date; review security advisories before upgrading.

## Workflow & CI Guidelines

- All workflows live in `.github/workflows/`.
- The `copilot-setup-steps.yml` workflow configures the cloud agent environment.
- CI should run on every push to `main` and on all pull requests.
