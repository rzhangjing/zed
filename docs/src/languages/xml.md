---
title: XML
description: "Configure XML language support in Zed, including language servers and formatting."
---

# XML

> **Removed in this fork.** XML support came from the [XML extension](https://github.com/sweetppro/zed-xml/), and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [tree-sitter-grammars/tree-sitter-xml](https://github.com/tree-sitter-grammars/tree-sitter-xml)

## Configuration

If you have additional file extensions that are not being automatically recognized as XML just add them to [file_types](../reference/all-settings.md#file-types) in your Zed settings:

```json [settings]
  "file_types": {
    "XML": ["rdf", "gpx", "kml"]
  }
```
