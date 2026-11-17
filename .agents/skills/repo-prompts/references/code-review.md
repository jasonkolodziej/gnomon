---
name: code-review
description: "Perform a thorough code review of the selected code or a pull request diff, checking for correctness, security, style, and test coverage."
disable-model-invocation: true
---
# Code Review

Review the code provided below (or the currently open file/diff) and produce structured feedback. Follow the guidelines in `.github/copilot-instructions.md`.

## Review Checklist

Evaluate each item and note findings (✅ pass / ⚠️ warning / ❌ issue):

- [ ] **Correctness** — Does the code do what it claims? Are there logic errors or off-by-one bugs?
- [ ] **Security** — Are there injection risks, unvalidated inputs, hardcoded secrets, or insecure defaults?
- [ ] **Error handling** — Are errors caught and handled gracefully? Are failure modes documented?
- [ ] **Performance** — Are there obvious inefficiencies (e.g., N+1 queries, unnecessary allocations)?
- [ ] **Readability** — Are names descriptive? Is the code self-explanatory?
- [ ] **Test coverage** — Are edge cases, error paths, and the happy path tested?
- [ ] **Documentation** — Are public APIs, config options, and non-obvious decisions documented?
- [ ] **Conventional commits** — Is the commit message properly formatted?

## Code / Diff to Review

${input:code:Paste the code, diff, or describe what to review}

## Additional Context

${input:context:PR description, ticket link, or any relevant background}
