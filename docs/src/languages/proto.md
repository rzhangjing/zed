---
title: Proto
description: "Configure Proto language support in Zed, including language servers and formatting."
---

# Proto

> **Removed in this fork.** Proto/proto3 (Protocol Buffers definition language) support came from the Proto extension, and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [coder3101/tree-sitter-proto](https://github.com/coder3101/tree-sitter-proto)
- Language Servers: [protobuf-language-server](https://github.com/lasorda/protobuf-language-server)

<!--
TBD: Clarify which language server(s) to use / Feature support.

## Setup

### Install protobuf-language-server

Install protobuf-language-server and make sure it's in your PATH:

```
go install github.com/lasorda/protobuf-language-server@latest
which protobuf-language-server
```

### Install ProtoLS

Install protols and make sure it's in your PATH:

```
cargo install protols
which protols
```

## Configuration

```json [settings]
"lsp": {
  "protobuf-language-server": {
    "binary": {
      "path": "protols"
    }
  }
}
```

## Formatting

ProtoLS supports formatting if you have `clang-format` installed.

```sh
# MacOS:
brew install clang-format
# Ubuntu
sudo apt-get install clang-format
# Fedora
sudo dnf install clang-tools-extra
```

To customize your formatting preferences, create a `.clang-format` file, e.g.:

```clang-format
IndentWidth: 4
ColumnLimit: 120
```

Or you can have zed directly invoke `clang-format` by specifying it as a [formatter](https://zed.dev/docs/reference/all-settings#formatter) in your settings:

```json [settings]
  "languages": {
    "Proto": {
      "format_on_save": "on",
      "tab_size": 4,
      "formatter": {
        "external": {
          "command": "clang-format",
          "arguments": ["-style={IndentWidth: 4, ColumnLimit: 0}"]
        }
      }
    },
  }
```
-->
