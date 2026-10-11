---
title: PowerShell
description: "Configure PowerShell language support in Zed, including language servers and formatting."
---

# PowerShell

> **Removed in this fork.** PowerShell support came from the community-maintained [Zed PowerShell extension](https://github.com/wingyplus/zed-powershell), and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [airbus-cert/tree-sitter-powershell](https://github.com/airbus-cert/tree-sitter-powershell)
- Language Server: [PowerShell/PowerShellEditorServices](https://github.com/PowerShell/PowerShellEditorServices)

## Setup

### Install PowerShell 7+ {#powershell-install}

- macOS: `brew install powershell/tap/powershell`
- Alpine: [Installing PowerShell on Alpine Linux](https://learn.microsoft.com/en-us/powershell/scripting/install/install-alpine)
- Debian: [Install PowerShell on Debian Linux](https://learn.microsoft.com/en-us/powershell/scripting/install/install-debian)
- RedHat: [Install PowerShell on RHEL](https://learn.microsoft.com/en-us/powershell/scripting/install/install-rhel)
- Ubuntu: [Install PowerShell on RHEL](https://learn.microsoft.com/en-us/powershell/scripting/install/install-ubuntu)
- Windows: [Install PowerShell on Windows](https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows)

The Zed PowerShell extension will default to the `pwsh` executable found in your path.

### Install PowerShell Editor Services (Optional) {#powershell-editor-services}

The Zed PowerShell extension will attempt to download [PowerShell Editor Services](https://github.com/PowerShell/PowerShellEditorServices) automatically.

If you want to use a specific binary, you can specify that in your Zed settings.json:

```json [settings]
  "lsp": {
    "powershell-es": {
      "binary": {
        "path": "/path/to/PowerShellEditorServices"
      }
    }
  }
```
