---
title: HTML
description: "Configure HTML language support in Zed, including language servers and formatting."
---

# HTML

> **Removed in this fork.** HTML support came from the HTML extension, and the extensions subsystem was removed from this fork. This page is kept for historical reference.

- Tree-sitter: [tree-sitter/tree-sitter-html](https://github.com/tree-sitter/tree-sitter-html)
- Language Server: [microsoft/vscode-html-languageservice](https://github.com/microsoft/vscode-html-languageservice)

## Formatting

By default Zed formats HTML with the `vscode-html-language-server` language server.

Configure formatting in Settings ({#kb zed::OpenSettings}) under Languages > HTML, or add to your settings file:

```json [settings]
  "languages": {
    "HTML": {
      "format_on_save": "off",
    }
  }
```

You can still trigger formatting manually with {#kb editor::Format} or by opening the [command palette](..//getting-started.md#command-palette) ({#kb command_palette::Toggle}) and selecting "Format Document".

### External Formatting

To use an external formatter such as [Prettier](https://prettier.io/) instead, configure it in Settings ({#kb zed::OpenSettings}) under Languages > HTML, or add to your settings file:

```json [settings]
  "languages": {
    "HTML": {
      "formatter": {
        "external": {
          "command": "prettier",
          "arguments": ["--stdin-filepath", "{buffer_path}"]
        }
      },
    }
  }
```

You can customize various [formatting options](https://code.visualstudio.com/docs/languages/html#_formatting) for `vscode-html-language-server` via your Zed `settings.json`:

```json [settings]
  "lsp": {
    "vscode-html-language-server": {
      "settings": {
        "html": {
          "format": {
            // Indent under <html> and <head> (default: false)
            "indentInnerHtml": true,
            // Disable formatting inside <svg> or <script>
            "contentUnformatted": "svg,script",
            // Add an extra newline before <div> and <p>
            "extraLiners": "div,p"
          }
        }
      }
    }
  }
```

## Using the Tailwind CSS Language Server with HTML

To get all the features (autocomplete, linting, etc.) from the [Tailwind CSS language server](https://github.com/tailwindlabs/tailwindcss-intellisense/tree/HEAD/packages/tailwindcss-language-server#readme) in HTML files, you need to configure the language server so that it knows about where to look for CSS classes by adding the following to your `settings.json`:

```json [settings]
{
  "lsp": {
    "tailwindcss-language-server": {
      "settings": {
        "experimental": {
          "classRegex": ["class=\"([^\"]*)\""]
        }
      }
    }
  }
}
```

With these settings, you will get completions for Tailwind CSS classes in HTML `class` attributes. Examples:

```html
<div class="flex items-center <completion here>">
  <p class="text-lg font-bold <completion here>">Hello World</p>
</div>
```

## See also

- [CSS](./css.md)
- [JavaScript](./javascript.md)
- [TypeScript](./typescript.md)
