---
name: repo-prompts
disable-model-invocation: true
description: Consolidated repository prompt workflows for code review, feature scaffolding, and test generation. Use this skill when the user wants a repository-specific review, a new feature implementation plan, or a test suite for existing code.
disable-model-invocation: true
---

# Repository Prompt Pack

This skill consolidates the repository's prompt workflows for common engineering tasks. Use it when the user wants help with code review, feature scaffolding, or writing tests for this repo.

## When to Use This Skill

Choose this skill for one of the following tasks:

- Review a code diff, file, or patch for correctness, security, and test coverage.
- Scaffold a new feature and create the implementation plus a matching test.
- Write a comprehensive test suite for an existing function, module, or feature.

When a task is narrower than the full repo, summarize the specific need and then execute the relevant section below.

## Required Repository Guidance

Follow the repo guidance in `.github/copilot-instructions.md` and the project conventions in this repository. Keep changes focused, document public API changes, and prefer deterministic tests.

## 1) Code Review Workflow

Use this workflow to review code or a diff.

### Inputs

- Code or diff to review: `${input:code:Paste the code, diff, or describe what to review}`
- Additional context: `${input:context:PR description, ticket link, or any relevant background}`

### Review Checklist

Evaluate each item and note findings as ✅ pass, ⚠️ warning, or ❌ issue:

- Correctness — Does the code do what it claims?
- Security — Are there injection risks, unvalidated inputs, hardcoded secrets, or insecure defaults?
- Error handling — Are failures handled gracefully and clearly?
- Performance — Are there obvious inefficiencies or repeated work?
- Readability — Are names and structure clear and maintainable?
- Test coverage — Are happy paths, edge cases, and error paths covered?
- Documentation — Are public APIs and decisions documented?
- Conventional commits — Is the commit message well-formed?

### Output Expectations

Provide a structured review with findings, severity, and concrete fixes or follow-up questions when the request is ambiguous.

## 2) New Feature Workflow

Use this workflow to scaffold or implement a new feature.

### Inputs

- Feature description: `${input:featureDescription:Describe the feature to implement (what it should do and why)}`
- Related files: `${input:relatedFiles:List any related files or leave blank}`
- Constraints: `${input:constraints:Any constraints or non-functional requirements}`

### Instructions

1. Clarify ambiguous requirements before writing code.
2. Implement the feature in the correct directory using existing conventions.
3. Add or update tests for the happy path and an error or edge case.
4. Update `README.md` when the change affects public behavior, APIs, or config.
5. Draft a conventional commit summary (`feat(scope): description`).

### Output Expectations

Deliver the implementation, the relevant tests, and any required documentation updates, while staying aligned with the repo's existing standards.

## 3) Test Writing Workflow

Use this workflow to generate or improve tests for code.

### Inputs

- Code or module under test: `${input:code:Paste the code to be tested, or describe the module/function}`
- Framework and language: `${input:framework:Test framework and language to use (leave blank to auto-detect)}`
- Additional context: `${input:context:Any mocking requirements, fixtures, or setup notes}`

### Instructions

1. Identify the public functions, methods, or endpoints that need coverage.
2. Cover the happy path, edge cases, and error paths.
3. Use the existing repository test framework when present; otherwise choose an appropriate framework for the detected language.
4. Keep tests deterministic and free from external timing or randomness unless mocked.
5. Use descriptive test names in specification form.

### Output Expectations

Provide the test suite, explain any assumptions, and ensure that the tests are targeted and meaningful rather than redundant.

## Invocation Guidance

This skill is intentionally a combined repository prompt pack. If the user request is narrow, focus only on the relevant workflow and avoid expanding scope unnecessarily.
