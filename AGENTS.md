# Agent Implementation Protocol

For every implementation task, follow this strict cycle:

1.  **Implement**: Code the feature or fix.
2.  **Verify**: Run existing tests (ex: `cargo test`) and add new integration/unit tests if applicable.
3.  **Documentation**: Update relevant `/docs/*.md` files to reflect architectural or usage changes.
4.  **Commit**: `git commit` only after tests pass and docs are updated.

**Rule**: Never commit broken code or undocumented features.


## 1. Philosophy & Guidelines

### Core Philosophy

- **Incremental progress over big bangs**: Break complex tasks into manageable stages.
- **Learn from existing code**: Understand patterns before implementing new features.
- **Clear intent over clever code**: Prioritize readability and maintainability.
- **Simple over complex**: Keep all implementations simple and straightforward - prioritize solving problems and ease of maintenance over complex solutions.

## Decision Philosophy

- I **hate to read 'optional'**, we must take informed decisions so offer me options, explain it to me with what value it brings.
- If I ask advice, it is for you to guide.offer me options not to ask me what my decisions because I don't know
- We are in **development phase**, breaking change is fine, no migration or fallback/legacy logic needed

## Testing Philosophy

- **I HATE MOCK tests**, either do unit or e2e, nothing inbetween. Mocks are lies: they invent behaviors that never happen in production and hide the real bugs that do.
- Test `EVERYTHING`. Tests must be rigorous. Our intent is ensuring a new person contributing to the same code base cannot break our stuff and that nothing slips by. We love rigour.
- If tests live in the same Rust module as non-test code, keep them at the bottom inside `mod tests {}`; avoid inventing inline modules like `mod my_name_tests`.
- Unless the user asks otherwise, run only the tests you added or modified instead of the entire suite to avoid wasting time.

### Eight Honors and Eight Shames

- **Shame** in guessing APIs, **Honor** in careful research.
- **Shame** in vague execution, **Honor** in seeking confirmation.
- **Shame** in assuming business logic, **Honor** in human verification.
- **Shame** in creating interfaces, **Honor** in reusing existing ones.
- **Shame** in skipping validation, **Honor** in proactive testing.
- **Shame** in breaking architecture, **Honor** in following specifications.
- **Shame** in pretending to understand, **Honor** in honest ignorance.
- **Shame** in blind modification, **Honor** in careful refactoring.

### Quality Standards

- **English Only**: usage of Chinese comments is strictly forbidden.
- **No Unnecessary Comments**: For simple, obvious code, let the code speak for itself.
- **Self-Documenting Code**: Prefer explicit types and clear naming over inline documentation.
- **Composition over Inheritance**: Favor functional patterns where applicable (Rust).
