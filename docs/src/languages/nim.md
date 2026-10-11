---
title: Nim
description: "Configure Nim language support in Zed, including language servers and formatting."
---

# Nim

> **Removed in this fork.** Nim support came from the community-maintained [Nim extension](https://github.com/foxoman/zed-nim), and the extensions subsystem was removed from this fork. This page is kept for historical reference.
Report issues to: [https://github.com/foxoman/zed-nim/issues](https://github.com/foxoman/zed-nim/issues)

- Tree-sitter: [alaviss/tree-sitter-nim](https://github.com/alaviss/tree-sitter-nim)
- Language Server: [nim-lang/langserver](https://github.com/nim-lang/langserver)

## Formatting

To use [arnetheduck/nph](https://github.com/arnetheduck/nph) as a formatter, follow the [nph installation instructions](https://github.com/arnetheduck/nph?tab=readme-ov-file#installation).

Configure formatting in Settings ({#kb zed::OpenSettings}) under Languages > Nim, or add to your settings file:

```json [settings]
  "languages": {
    "Nim": {
      "formatter": {
        "external": {
          "command": "nph",
          "arguments": ["-"]
        }
      }
    }
  }
```
