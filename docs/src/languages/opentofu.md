---
title: OpenTofu
description: "Configure OpenTofu language support in Zed, including language servers and formatting."
---

# OpenTofu

> **Removed in this fork.** OpenTofu support came from the [OpenTofu extension](https://github.com/ashpool37/zed-extension-opentofu), and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [tree-sitter-grammars/tree-sitter-hcl](https://github.com/tree-sitter-grammars/tree-sitter-hcl)
- Language Server: [opentofu/tofu-ls](https://github.com/opentofu/tofu-ls)

## Configuration

To associate `.tf` and `.tfvars` files with OpenTofu, add this to your settings.json:

```json
"file_types": {
  "OpenTofu": ["tf"],
  "OpenTofu Vars": ["tfvars"]
},
```

See the [full list of server settings here](https://github.com/opentofu/tofu-ls/blob/main/docs/SETTINGS.md).
