---
title: Terraform
description: "Configure Terraform language support in Zed, including language servers and formatting."
---

# Terraform

> **Removed in this fork.** Terraform support came from the [Terraform extension](https://github.com/zed-extensions/terraform), and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [tree-sitter-grammars/tree-sitter-hcl](https://github.com/tree-sitter-grammars/tree-sitter-hcl)
- Language Server: [hashicorp/terraform-ls](https://github.com/hashicorp/terraform-ls)

## Configuration

<!--
TBD: Add example using `rootModulePaths` to match upstream example https://github.com/hashicorp/terraform-ls/blob/main/docs/SETTINGS.md#vs-code
-->

The Terraform language server can be configured in your `settings.json`, e.g.:

```json [settings]
{
  "lsp": {
    "terraform-ls": {
      "initialization_options": {
        "experimentalFeatures": {
          "prefillRequiredFields": true
        }
      }
    }
  }
}
```

See the [full list of server settings here](https://github.com/hashicorp/terraform-ls/blob/main/docs/SETTINGS.md).
