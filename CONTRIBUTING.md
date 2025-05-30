# Contributing to `codetypo-lsp`

This document contains information on how to contribute to the `codetypo-lsp` crate, which is the implementation of the Language Server Protocol (LSP) server.

The source code of `codetypo-lsp` can be found in the [crates/codetypo-lsp/](./crates/codetypo-lsp/) directory.

Make sure you have installed the rust toolchain with [rustup](https://rust-lang.github.io/rustup/index.html).

## Setting up your development environment

### How to run tests

```sh
cargo test
```

### How to run the LSP server locally for development

First, compile the LSP server:

```sh
cargo build
```

Next, configure your editor to launch the LSP server.

#### Running locally in Neovim

For example, in Neovim, you can use the [sample neovim LSP config](docs/neovim-lsp-config.md) in the following way:

```lua
-- Set the path of the LSP server to the compiled binary on your disk
require('lspconfig').codetypo_lsp.setup({
  -- ... other settings from the example
  cmd = { "/Users/me/git/codetypo-lsp/target/debug/codetypo-lsp" },
})
```

To test your configuration, you can use the following methods:

- open e.g. [crates/codetypo-lsp/tools/test.txt](crates/codetypo-lsp/tools/test.txt). You should see errors when opening the file.
- you can also run `:LspInfo` in Neovim to see the status of the LSP server.
- you can run `cargo clean` in the root directory of the project. After this, you can make sure Neovim displays errors when opening a file. Once finished, run `cargo build` to fix this

<!-- markdownlint-disable-file line-length -->
