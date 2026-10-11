---
title: Biome
description: "Configure Biome language support in Zed, including language servers and formatting."
---

# Biome

> **Removed in this fork.** [Biome](https://biomejs.dev/) support came from the community-maintained [Biome extension](https://github.com/biomejs/biome-zed), and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Language Server: [biomejs/biome](https://github.com/biomejs/biome)

## Biome Language Support

The Biome extension includes support for the following languages:

- JavaScript
- TypeScript
- JSX
- TSX
- JSON
- JSONC
- Vue.js
- Astro
- Svelte
- CSS

## Configuration

By default, the `biome.json` file is required to be in the root of the workspace.

```json
{
  "$schema": "https://biomejs.dev/schemas/1.8.3/schema.json"
}
```

For a full list of `biome.json` options see [Biome Configuration](https://biomejs.dev/reference/configuration/) documentation.

See the [Biome Zed Extension README](https://github.com/biomejs/biome-zed) for a complete list of features and configuration options.
