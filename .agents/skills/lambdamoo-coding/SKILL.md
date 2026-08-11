---
name: lambdamoo-coding
description: Check and format LambdaMOO source with moo-lsp-rs. Use when creating, editing, reviewing, validating, or formatting .moo files, diagnosing LambdaMOO syntax or semantic errors, or verifying LambdaMOO code before completion.
---

# Check and format LambdaMOO

Confirm `moo-lsp-rs` is available on `PATH` before relying on it.

## Validate changes

Run the checker after creating or modifying `.moo` files:

```sh
moo-lsp-rs check path/to/file.moo
```

Pass multiple files or a directory when appropriate. Use `-` for source supplied on stdin. Fix every error before completing the task and report warnings that remain relevant.

Use JSON only when structured processing is useful:

```sh
moo-lsp-rs check --json path/to/file.moo
```

Treat exit status `0` as successful validation, `1` as reported diagnostics, and `2` as a usage or I/O failure. Add `--deny-warnings` only when the task or project requires warnings to fail validation.

## Format safely

Check formatting without modifying files:

```sh
moo-lsp-rs format --check path/to/file.moo
```

Preview one formatted file on stdout:

```sh
moo-lsp-rs format path/to/file.moo
```

Use `--write` only when file modification is authorized:

```sh
moo-lsp-rs format --write path/to/file.moo
```

Never force formatting of invalid source. Re-run `moo-lsp-rs check` after formatting and before reporting completion.
