---
name: write-tests
description: "Generate a comprehensive test suite for the selected code, covering unit tests, edge cases, and error paths."
disable-model-invocation: true
---
# Write Tests

Generate a comprehensive test suite for the code provided below. Follow all guidelines in `.github/copilot-instructions.md`.

## Instructions

1. **Identify all public functions/methods/endpoints** that need to be tested.
2. For each, write tests that cover:
   - **Happy path** — expected input produces expected output.
   - **Edge cases** — empty inputs, boundary values, large/small numbers, special characters.
   - **Error paths** — invalid input, missing dependencies, network/IO failures.
3. Use the **existing test framework** already present in the repository. If none exists, choose a popular framework for the detected language.
4. Make tests **deterministic** — no reliance on system clock, random values, or external services unless properly mocked.
5. Use **descriptive test names** that read like specifications: `it("returns 404 when user is not found")`.

## Code to Test

${input:code:Paste the code to be tested, or describe the module/function}

## Test Framework / Language

${input:framework:Test framework and language to use (leave blank to auto-detect)}

## Additional Context

${input:context:Any mocking requirements, fixtures, or setup notes}
