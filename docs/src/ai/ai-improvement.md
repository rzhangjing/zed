---
title: Feedback and Training Data - Zed
description: Understand what opt-in AI feedback and edit prediction exclusions do in this fork.
---

# Feedback and Training Data

Normal AI requests are not retained by Zed, because this fork does not route AI
requests through Zed. Every request goes from your machine to a provider you
configured, under that provider's terms.

AI features in Zed include:

- [Agent Panel](./agent-panel.md)
- [Edit Prediction](./edit-prediction.md)

For the broader request path and provider data boundaries, see
[AI Privacy](./privacy-and-security.md).

## Response Ratings and Feedback {#ai-feedback-with-ratings}

You can still rate an AI response in a thread. In this fork a rating is kept
locally in the thread UI only: the code that would have sent the conversation
thread to Zed was removed, so your messages, AI responses, and thread metadata do
not leave your machine.

Conversation-thread feedback upload, its server-side storage, and the
anonymization pipeline that went with it were all removed.

## Edit Prediction Training Data {#edit-predictions}

There is no Edit Prediction training-data collection in this fork. The opt-in
toggle, the open-source-license and glob gating, and the upload were removed
together with the Zed-hosted edit prediction model. Edit prediction now runs
against a provider you configure; see [Edit Prediction](./edit-prediction.md).

### File Exclusions {#file-exclusions}

`edit_predictions.disabled_globs` still works: files matching these globs are
never used as edit prediction context. Certain files are always excluded:

```json [settings]
{
  "edit_predictions": {
    "disabled_globs": [
      "**/.env*",
      "**/*.pem",
      "**/*.key",
      "**/*.cert",
      "**/*.crt",
      "**/.dev.vars",
      "**/secrets.yml"
    ]
  }
}
```

You can explicitly exclude additional paths or file extensions by adding them to
[`edit_predictions.disabled_globs`](https://zed.dev/docs/reference/all-settings#edit-predictions)
in your Zed settings file ([how to edit](../configuring-zed.md#settings-files)):

```json [settings]
{
  "edit_predictions": {
    "disabled_globs": ["secret_dir/*", "**/*.log"]
  }
}
```

## Removed in This Fork {#removed}

- Zed-hosted model commitments, including the provider safety-retention exception
  for designated models such as Anthropic's Covered Models
- Conversation-thread feedback upload and its stored datasets
- Edit Prediction training-data collection, the `zeta` training dataset, and the
  fine-tuned hosted model
- Zed Business data-sharing controls for organizations

## Applicable Terms {#applicable-terms}

See the [Zed Terms of Service](https://zed.dev/terms) for more.
