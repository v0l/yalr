# YALR - Agent Instructions

## Overview

YALR (Yet Another LLM Router) is an async LLM router with load balancing, provider abstraction, and future web UI support.

## Development Guidelines

1. **Async First**: All I/O operations must be async using tokio
2. **Trait-Based**: Use traits for all abstractions to enable testing and extension
3. **Streaming Support**: All providers must support streaming responses
4. **Error Handling**: Use `thiserror` for library errors, proper error propagation
5. **Logging**: Use `tracing` for structured logging
6. **Testing**: Write unit tests using `#[cfg(test)] mod tests {}` inline within the source file, not as separate test files

### Testing Convention

- All tests should be written as inline modules: `#[cfg(test)] mod tests { ... }`
- Place the test module at the bottom of the file being tested
- Do not create separate `*_test.rs` files

## Implementation Plan

See [PLAN.md](./PLAN.md) for the full implementation plan.