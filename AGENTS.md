# AGENTS.md — deerpunk

This file defines the default engineering rules for human and AI contributors working on deerpunk. Follow it unless a more specific, applicable instruction file overrides it. If instructions conflict, explain the conflict and follow the more specific rule only when it is clearly intended to override this one.

The project is under active prototyping. These rules aim to preserve development speed while keeping the code understandable, modular, and maintainable. Do not turn every prototype task into a broad refactor.

## 1. Core principles

1. **Understand before changing.** Inspect the relevant code, call sites, crate boundaries, and existing patterns before designing a solution. Do not assume the repository has a particular layout, command, subsystem, or convention until you verify it.
2. **Prefer clear, idiomatic Rust.** Optimize for readability and explicit contracts before cleverness, compression, or speculative generality.
3. **Preserve working behavior.** Make the smallest change that correctly addresses the problem. Avoid unrelated rewrites and do not replace a working implementation merely because a different design looks cleaner.
4. **Fix the cause, not the symptom.** Trace errors to the layer that owns the invariant or responsibility. Do not add broad fallbacks, retries, defaults, or error suppression simply to make a failure disappear.
5. **Prototype deliberately.** Temporary shortcuts are acceptable when they unblock iteration, but they must be visible, bounded, and not misrepresented as complete production-ready behavior.
6. **Respect architecture boundaries.** Keep dependencies flowing in the intended direction. Do not spread format-specific, renderer-specific, platform-specific, or backend-specific details across unrelated subsystems.
7. **Design for the next developer.** A contributor should be able to understand the intent, assumptions, ownership, and failure behavior from the code and nearby documentation—not from the original AI conversation.
8. **Be honest about verification.** Never claim that code compiles, tests pass, a feature is complete, or behavior is correct unless the relevant check or evidence supports that claim.

## 2. Before making a change

- Inspect the relevant files and their callers before editing.
- Check for existing abstractions, helper functions, error types, traits, conventions, and tests before introducing new ones.
- Follow the repository's actual crate structure and naming conventions. Do not invent a new architecture based on assumptions.
- Identify which subsystem owns the behavior and make the change there where practical.
- Consider the impact on public APIs, serialization, asset compatibility, game code, client/server behavior, and platform integrations where relevant.
- If the requested change conflicts with the current architecture, explain the conflict and choose the smallest coherent solution.
- Ask for clarification only when an ambiguity materially affects correctness, compatibility, or the design. Otherwise, make a reasonable local decision and state any important assumption.

## 3. Rust style and readability

- Follow `rustfmt` and the existing project style.
- Prefer descriptive names that communicate domain meaning. Avoid vague names such as `data`, `thing`, `manager`, or `helper` when a more precise name is practical.
- Keep functions focused on one coherent responsibility. Extract helpers when they clarify the algorithm or isolate a meaningful step—not merely to reduce line count.
- Prefer explicit control flow over dense expressions when explicit code is easier to understand.
- Avoid deeply nested conditionals, oversized functions, boolean parameters with unclear meaning, and unnecessary layers of indirection.
- Use enums and domain-specific types when they make invalid states harder to represent. Do not create wrapper types that add no meaningful guarantees or vocabulary.
- Prefer borrowing over cloning when it is natural and clear. Do not contort ownership or introduce complicated lifetimes merely to avoid a small, justified clone.
- Do not use `clone()` to paper over an ownership problem without first understanding why ownership is difficult. Consider whether the data model, lifetime, or responsibility boundary is wrong.
- Avoid unnecessary allocation in hot paths, but do not sacrifice clarity for unmeasured micro-optimizations.
- Avoid premature generic frameworks, plugin layers, registries, trait hierarchies, and configuration systems. Introduce abstractions when there is a demonstrated need or a clear architectural boundary.
- Do not use macros when ordinary functions, traits, or types are clearer.
- Do not suppress compiler or Clippy warnings without understanding them. Keep any necessary lint allowance as narrow as possible and explain non-obvious exceptions.
- Do not leave commented-out implementations, stale TODOs, debug prints, temporary hacks, or dead code behind unless they are intentional and clearly explained.

### Comments and documentation language

**This repository (deerpunk) is English-only** for anything that ships with the engine:

- Rust doc comments (`///`, `//!`), module docs, and `AGENTS.md` / in-repo technical markdown under `crates/`
- Inline `//` comments and `// SAFETY:` notes
- Identifiers, log messages, and user-visible engine diagnostics (unless a localization layer already owns that string)

Game-specific repositories (e.g. darkdemo) may use another language for *their* docs and comments; do **not** add Russian (or other non-English) comments or Rustdoc inside deerpunk when implementing a game feature—keep game-only wording in the game repo.

Match the established language of user-facing strings if the project already has a localization convention.

Comments should explain **why**, invariants, trade-offs, assumptions, or non-obvious behavior. Do not narrate what the next line plainly does.

Bad:
```rust
// Increment the counter.
counter += 1;
```

Better:
```rust
// Keep the sequence monotonic because clients use it to reject stale updates.
sequence += 1;
```

Do not add comments that merely restate the code. Update comments when behavior changes.

## 4. API and documentation

- Document public library APIs when their purpose, contract, invariants, or correct usage is not obvious.
- Public structs, enums, traits, functions, and non-obvious public fields should have useful Rustdoc comments (`///`). Public modules should be documented where that helps explain their role.
- Explain constraints, units, ownership expectations, error behavior, and lifecycle requirements where relevant.
- Document non-obvious internal decisions near the code or in the appropriate architecture document.
- Add examples for APIs that are difficult to use correctly or are central integration points. Examples must reflect real APIs and should compile where practical.
- Avoid writing documentation that promises behavior the implementation does not provide.
- Do not document every local variable or obvious private helper. Documentation should reduce the effort needed to understand and safely use the code.
- When changing a public API, update affected documentation and call sites. Mention compatibility implications in the final report.
- Use the repository's existing documentation structure where possible. Do not create a new documentation hierarchy for a single trivial note.

## 5. Errors, invariants, and failure handling

- Handle expected runtime failures explicitly and preserve useful diagnostic context.
- Use `Result` for operations that can fail for expected reasons, such as loading assets, parsing external data, initializing backends, or communicating across subsystem boundaries.
- Choose error types and messages that help locate the failure. Add context at meaningful boundaries; avoid repeatedly wrapping the same error with redundant messages.
- Do not silently ignore errors. If an error is intentionally ignored, document why ignoring it is safe.
- Do not convert errors into success-shaped defaults, empty data, zero values, or fake objects unless that behavior is explicitly part of the contract.
- Do not build long fallback chains that conceal a broken primary path. A fallback is appropriate only when it is a deliberate, bounded product behavior with a clear reason and observable outcome.
- Do not catch an error only to log it and continue in an invalid state. Either recover safely, propagate the error, or stop the affected operation with a clear failure.
- Preserve the rest of the application when a recoverable operation fails. For example, a malformed optional asset should not crash an unrelated subsystem if the engine has a defined way to reject that asset. However, do not pretend that the failed asset loaded successfully.
- Distinguish recoverable content errors, subsystem initialization failures, violated programmer invariants, and process-level failures.
- Use `unwrap()`, `expect()`, and `panic!()` only when the invariant is genuinely established or the state is intentionally unrecoverable. Add a short explanation for non-obvious invariants. Do not use them for ordinary external input or expected runtime failures.
- Do not use `unreachable!()` merely because a case is unexpected. If external input or evolving state can reach it, handle the case explicitly.
- Do not add catch-all recovery or `catch_unwind` as a substitute for correct error handling. Use panic containment only at a justified isolation boundary.
- Do not introduce a new error-handling dependency or framework without explaining why existing mechanisms are insufficient.

Preferred pattern:
```rust
fn load_world(path: &Path) -> Result<World, WorldLoadError> {
    let bytes = std::fs::read(path).map_err(WorldLoadError::Read)?;
    parse_world(&bytes).map_err(WorldLoadError::Parse)
}
```

Avoid:
```rust
fn load_world(path: &Path) -> World {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| parse_world(&bytes).ok())
        .unwrap_or_default()
}
```

The first example makes failure visible. The second silently converts failure into a potentially misleading empty world.

## 6. Unsafe Rust and FFI

- Keep `unsafe` code confined to established low-level, platform, or FFI boundaries wherever practical. Do not spread unsafe operations through gameplay or high-level engine code.
- Before adding `unsafe`, check whether a safe API or a narrower wrapper can solve the problem.
- Every non-obvious unsafe block must have a `// SAFETY:` comment explaining why its requirements are upheld. The comment must describe the actual invariants, not simply say that the operation is safe.
- Keep unsafe blocks as small as possible. Do not mark an entire function unsafe when only a small operation requires it, unless the function's public contract truly requires that.
- Do not use unsafe to bypass borrow-checker issues without understanding the ownership model.
- At FFI boundaries, validate pointers, lengths, enum values, handles, alignment, and lifetimes as applicable. Do not assume foreign input is valid.
- Do not pass Rust-specific types such as `String`, `Vec<T>`, trait objects, or ordinary Rust references across a C ABI unless the interface explicitly defines and safely implements the representation. Use appropriate FFI-safe representations and ownership rules.
- Do not assume Rust's native ABI is stable across independently compiled dynamic libraries. Verify the actual linking and ABI contract before designing interfaces.
- Document who allocates and frees memory, how long pointers remain valid, which side owns handles, how errors cross the boundary, and whether callbacks can re-enter the engine.
- Keep FFI conversion and validation near the boundary. Convert external representations into safe internal types before ordinary engine logic uses them.
- Do not impose a blanket ban on FFI or unsafe where the project genuinely needs integration with native libraries; isolate and justify it instead.

## 7. Crate boundaries and architecture

- Treat crate boundaries as intentional API and dependency boundaries, not merely folders.
- Before adding a dependency between crates, check whether it creates a cycle, leaks implementation details, or reverses the intended dependency direction.
- Keep shared contracts in the existing shared/API layer if the repository has one. Do not duplicate struct definitions across crates to avoid using the proper shared contract.
- Avoid making a low-level crate depend on a high-level game, UI, or application crate.
- Keep platform-specific code, third-party backend APIs, file-format parsing, gameplay rules, rendering, audio, physics, and networking separated according to the repository's established architecture.
- Do not let a backend's types become the public vocabulary of unrelated systems unless that coupling is deliberate.
- Convert external formats into engine-owned representations at appropriate boundaries. Do not make every subsystem understand a file format just because one subsystem loads it.
- Avoid "god" modules and "manager" types that accumulate unrelated responsibilities.
- Do not split code into extra crates just to appear modular. A new crate should provide a meaningful boundary, independent reuse/build needs, or a clear ownership benefit.
- Do not merge distinct subsystems merely to make imports shorter.
- Before introducing an abstraction, explain what variation or coupling it manages. If it has no concrete purpose yet, prefer a direct implementation.

## 8. Engine-specific engineering guidance

The following are architectural goals, not assumptions about what is already implemented. Inspect the repository before applying them.

### Rendering and assets
- Keep file-format parsing separate from runtime rendering where practical.
- Validate sizes, offsets, indices, counts, and references when parsing maps, models, textures, and other external assets.
- Treat asset files as untrusted input. Avoid unchecked indexing, integer overflow, unbounded allocations, and recursion without limits.
- Keep map-format-specific concepts out of general renderer APIs unless they are truly part of the engine's intended abstraction.
- Do not introduce visibility, batching, caching, or optimization frameworks speculatively. Measure or identify a concrete requirement first.
- Preserve intentional compatibility with GoldSrc-style assets and tools where the project requires it. Do not silently change file interpretation or coordinate conventions.

### Simulation, physics, and movement
- Keep authoritative gameplay/simulation rules distinct from presentation where the architecture supports that separation.
- Make coordinate conventions, units, collision assumptions, and timing semantics explicit at subsystem boundaries.
- When integrating physics or movement code, verify the conventions of both systems rather than silently changing the engine's established origin, velocity, or timestep semantics.
- Test edge cases in collision, movement, and numerical calculations when they are relevant to the change.
- Avoid mixing engine-specific movement semantics with a physics library's internal representation throughout the codebase. Use a clear adapter or boundary when needed.

### Client, server, and replication
- Preserve the separation between client-side presentation and authoritative server simulation where the architecture calls for it.
- Distinguish a client request from an authoritative state update and from a one-shot gameplay event.
- Validate client-originated requests on the authoritative side; do not treat client claims as proof of permission, position, or outcome.
- For replicated state, consider initial state on spawn or re-entry, changed state, late joining, loss, ordering, and resynchronization as applicable.
- Do not design a general-purpose networking framework for hypothetical future requirements when a small explicit protocol is enough. Do not hardwire current in-process communication in a way that makes later transport separation unnecessarily difficult.
- Keep network identifiers and wire representations separate from local implementation details where appropriate.
- Do not send every field every tick by default. Choose update frequency and delivery semantics based on the field's behavior and correctness requirements.

### Audio and other backends
- Keep backend-specific APIs behind the existing abstraction where practical.
- Do not add a second abstraction layer that merely forwards every backend call without providing isolation, portability, ownership, or a useful contract.
- Make resource ownership and lifecycle explicit, especially for native handles and callback-driven APIs.
- Handle backend initialization and device/resource failures visibly. Do not report successful initialization when the backend is unavailable.

## 9. Prototyping and technical debt

- Speed matters during prototyping. A small direct implementation is preferable to a speculative framework.
- A prototype may omit polish or broad generality, but it must not misrepresent unsupported behavior as implemented.
- Mark meaningful temporary limitations with a specific `TODO` or issue reference where the project uses issue tracking. Explain what is missing and what would be required to complete it.
- Avoid vague TODOs such as `TODO: improve this`. State the missing behavior, risk, or next action.
- Keep shortcuts local. A temporary limitation in one parser or backend should not force unrelated systems to adopt the same limitation.
- Do not add fake data, no-op implementations, empty success results, or dummy fallbacks merely to satisfy the compiler unless the stub is explicitly part of a known staged integration and is clearly labelled.
- Do not over-engineer a prototype in anticipation of unconfirmed requirements. At the same time, avoid shortcuts that make the next necessary step disproportionately difficult.
- If the clean solution is too large for the current task, implement a coherent smaller step and state the limitation rather than building a tangled partial framework.
- Do not perform broad unrelated cleanup while fixing a focused issue. Mention adjacent problems separately if they are important.

## 10. Dependencies and third-party code

Before adding a dependency:
- Explain what capability it provides and why the project needs it now.
- Check whether an existing dependency or local abstraction already provides the capability.
- Consider maintenance status, license, platform support, compile-time/build impact, security, and integration cost where relevant.
- Prefer a focused, established dependency over a large framework when both solve the problem adequately.
- Do not add dependencies for trivial helpers that are easy to implement clearly and safely.
- Do not copy substantial third-party implementation code without checking its license and attribution requirements.
- Do not add a dependency merely because it is fashionable or appears in another engine.

If a dependency is added, summarize the reason and any meaningful trade-offs in the final report.

## 11. Performance and allocation

- Do not optimize based only on intuition. Identify a measured bottleneck, a clear complexity problem, or a known hot path.
- Avoid needless allocations, cloning, formatting, logging, and lock contention in per-frame or per-entity paths.
- Prefer clear algorithms with appropriate complexity. Avoid introducing caches or pools without a lifecycle and invalidation strategy.
- Be explicit about work performed per frame, per tick, per entity, or per client when that distinction matters.
- Do not sacrifice correctness or readability for speculative micro-optimizations.
- When making a performance claim, state what was measured and under what conditions. Do not infer performance from a successful build or a single FPS counter alone.

## 12. Testing and verification

Tests are especially valuable for:
- Parsers and asset loaders, including malformed and truncated input.
- Numerical algorithms, collision, movement, and coordinate conversions.
- Serialization, protocol messages, replication, and state transitions.
- Bug fixes where a regression test can reproduce the original failure.
- Core invariants and tricky boundary conditions.

Guidelines:
- Add focused tests for critical behavior when practical. Tests should verify observable behavior, not merely mirror the implementation.
- Prefer small deterministic tests over brittle integration tests when both give useful coverage.
- Do not add tests that only assert a constant or restate a trivial line without protecting a meaningful contract.
- If a bug is reproducible, try to add a regression test. If that is impractical, explain why.
- Do not delete or weaken a test merely to make a change pass without understanding what it protects.
- Do not claim a test was run if it was not run.
- Discover the workspace layout and existing test commands before selecting commands. Do not assume every crate can or should run every check independently.
- Run `cargo fmt --all -- --check`, relevant Clippy checks, and relevant tests where supported and practical. If formatting changes are appropriate, run formatting rather than only checking it.
- Run `cargo doc` or documentation tests when changing important public APIs or Rustdoc examples, when practical.
- If a command cannot run, state the concrete reason (for example, missing native SDK, unavailable asset, platform limitation, or pre-existing build failure).
- Distinguish failures introduced by the change from failures that already existed.

## 13. Working with the repository and tools

- Inspect existing instructions (`AGENTS.md`, Cursor rules, README, contributing notes, crate-level docs) before making changes.
- Do not assume the current working tree is clean. Avoid overwriting unrelated user changes.
- Do not revert, reformat, or rewrite unrelated files unless explicitly asked.
- Do not run destructive commands or mass-edit files without understanding their scope.
- Do not invent command names, test targets, environment variables, asset paths, or CI jobs. Inspect manifests, scripts, and documentation first.
- Use the repository's established toolchain and conventions. Do not update Rust toolchains or large dependency sets as an incidental part of a feature task.
- When generated files are involved, identify the source of truth and update the appropriate source rather than editing generated output alone.
- Do not conceal uncertainty. If the repository does not provide enough information to verify a claim, say so.

## 14. Preferred problem-solving examples

### Example A: Fixing an asset-loading crash

Preferred:
1. Reproduce or locate the failing input and identify the exact unchecked assumption.
2. Determine whether the file is invalid, the parser is wrong, or the caller violates a contract.
3. Validate the relevant bounds and return a contextual parse error for malformed input.
4. Add a focused regression test if practical.
5. Preserve successful behavior for valid assets.

Avoid:
- Wrapping the entire loader in a catch-all panic handler.
- Returning an empty asset on every error.
- Adding several fallback parsers without evidence that multiple formats are intended.
- Rewriting the entire asset pipeline to fix one bounds check.

### Example B: A backend initialization failure

Preferred:
- Report which backend/resource failed and preserve the underlying error.
- Follow the project's intended failure policy: fail initialization if the subsystem is required, or disable only that feature if optional operation is explicitly supported.
- Keep the rest of the application running only when it can remain in a valid, clearly defined state.

Avoid:
- Claiming audio or rendering initialized when it did not.
- Silently substituting a dummy backend.
- Retrying indefinitely without a policy.
- Swallowing the error and leaving partially initialized state.

### Example C: Adding a new public API

Preferred:
- Find the existing layer that should own the API.
- Choose names and types that express the contract.
- Document non-obvious semantics, units, errors, ownership, and lifecycle.
- Add tests or examples for tricky behavior.
- Keep the API as small as possible until actual use cases justify more.

Avoid:
- Exposing backend implementation types through a general API without a reason.
- Adding multiple speculative configuration options.
- Creating a new trait for a single implementation with no concrete need.
- Publishing an API whose behavior is still unclear.

### Example D: Fixing a borrow-checker problem

Preferred:
- Understand which value owns the data and how long it needs to live.
- Reconsider whether the function should borrow, take ownership, or return a value.
- Narrow the scope of borrows or split logically independent operations.
- Use cloning when it is the clearest correct choice and its cost is acceptable.

Avoid:
- Sprinkling `.clone()` everywhere until the code compiles.
- Introducing `Rc<RefCell<_>>`, `Arc<Mutex<_>>`, or unsafe pointers automatically as an escape hatch.
- Restructuring several crates before establishing that the current ownership boundary is the problem.

### Example E: Prototyping a feature

Preferred:
- Implement the smallest coherent end-to-end path.
- Clearly identify what works, what is stubbed, and what is not yet handled.
- Keep the implementation easy to replace or extend at the actual boundary.
- Add tests for critical invariants even if the feature is not complete.

Avoid:
- Building a general framework for imagined future features.
- Filling the code with placeholders that look like completed behavior.
- Hiding limitations behind default values or successful return codes.
- Expanding the task into an unrelated refactor.

### Example F: Adding a dependency

Preferred:
- Explain the missing capability and compare a reasonable existing option or a small local implementation.
- Verify license, platform support, maintenance, and integration constraints as relevant.
- Wrap the dependency behind a subsystem boundary if that prevents unnecessary coupling.

Avoid:
- Adding a crate for a trivial helper without considering the cost.
- Exposing dependency types everywhere by default.
- Replacing an existing library without a demonstrated benefit.

## 15. Completion report

At the end of a task, provide a concise report that states:

1. **Changed:** the main files or subsystems changed and the behavior implemented.
2. **Design:** important trade-offs, assumptions, or architecture decisions, if any.
3. **Verification:** exact checks, tests, and documentation commands run, and whether they passed.
4. **Not verified:** checks that were not run and the reason.
5. **Limitations / debt:** remaining stubs, known limitations, or follow-up work introduced by the change.
6. **Dependencies:** any new dependencies and why they were needed.

Do not list routine details that add no value. Do not claim that unrun checks passed. If no meaningful architectural decision or new technical debt was involved, say so briefly or omit that section.

## 16. Final decision rule

When several solutions are correct, prefer the one that:
- preserves existing working behavior;
- has the clearest ownership and responsibility boundaries;
- communicates errors rather than hiding them;
- uses the smallest useful abstraction;
- is straightforward to test and document;
- leaves the code easier for another developer to maintain;
- supports the current prototype without needlessly blocking future development.

Do not follow these rules mechanically when doing so would make the code less correct or less clear. If an exception is necessary, keep it narrow and explain the reason.
