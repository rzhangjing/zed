---
title: GDScript
description: "Configure GDScript language support in Zed, including language servers and formatting."
---

# GDScript

> **Removed in this fork.** Godot [GDScript](https://gdscript.com/) support came from the community-maintained [GDScript extension](https://github.com/GDQuest/zed-gdscript), and the extensions subsystem was removed from this fork. This page is kept for historical reference.
Report issues to: [https://github.com/GDQuest/zed-gdscript/issues](https://github.com/GDQuest/zed-gdscript/issues)

- Tree-sitter: [PrestonKnopp/tree-sitter-gdscript](https://github.com/PrestonKnopp/tree-sitter-gdscript) and [PrestonKnopp/tree-sitter-godot-resource](https://github.com/PrestonKnopp/tree-sitter-godot-resource)
- Language Server: [gdscript-language-server](https://github.com/godotengine/godot)

## Pre-requisites

You will need:

- [Godot](https://godotengine.org/download/).
- netcat (`nc` or `ncat`) on your system PATH.

## Setup

1. Inside your Godot editor, open Editor Settings, look for `Text Editor -> External` and set the following options:
   - Exec Path: `/path/to/zed`
   - Exec Flags: `{project} {file}:{line}:{col}`
   - Use External Editor: "✅ On"
2. Open any \*.gd file through Godot and Zed will launch.

## Usage

When Godot is running, the GDScript extension will connect to the language server provided by the Godot runtime and will provide `jump to definition`, hover states when you hold Ctrl/cmd and other language server features.
