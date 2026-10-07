---
title: LLM Providers - Zed
description: Choose how Zed gets language models: API access, OpenAI-compatible gateways, or local models.
---

# LLM Providers

Use this page to choose which models power [the Zed Agent](./zed-agent.md) and
other Zed-owned AI features, including Git commit generation, thread summaries,
and similar model-backed features.

Model access paths do not configure [External Agents](./external-agents.md) or
[Terminal Threads](./terminal-threads.md). External Agents and Terminal Threads
usually own their own model access, auth, and configuration.

## Choose a Model Access Path {#choose-a-model-access-path}

| Model access path                                   | Best when                                                    | Source of truth   |
| --------------------------------------------------- | ------------------------------------------------------------ | ----------------- |
| [Use API Access](./use-api-access.md)               | You have provider API access, credits, or usage billing      | Use API Access    |
| [Use a Gateway](./use-a-gateway.md)                 | You route through an OpenAI-compatible gateway               | Use a Gateway     |
| [Use a Local Model](./use-a-local-model.md)         | You run models locally or self-hosted                        | Use a Local Model |

This fork has no Zed-hosted models and no subscription sign-in. Model access is
always one of the paths above, or a subscription owned by an External Agent or
terminal CLI. See [Use an Existing Subscription](./use-an-existing-subscription.md).

Use the setup pages for provider-specific details. See [Agents](./agents.md) for
the difference between the Zed Agent, External Agents, and Terminal Threads.

## Supported Providers {#supported-providers}

Zed can talk to these providers directly:

- [OpenAI](./use-api-access.md#openai)
- [DeepSeek](./use-api-access.md#deepseek)
- [OpenAI-compatible endpoints](./use-api-access.md#openai-compatible)
- [Ollama](./use-a-local-model.md#ollama)
- [LM Studio](./use-a-local-model.md#lm-studio)
- [llama.cpp](./use-a-local-model.md#llama-cpp)

Anthropic, Google AI, GitHub Copilot, and the Zed-hosted model service were
removed from this fork. Claude, Codex, Copilot, Cursor, and similar tools are
still usable through [External Agents](./external-agents.md) and
[Terminal Threads](./terminal-threads.md), where the harness owns its own model
access.

## Edit Prediction {#edit-prediction}

[Edit Prediction](./edit-prediction.md) has its own provider setup under `edit_predictions`. LLM providers on this page apply to model-backed Zed AI features such as Zed Agent, Git commit generation, and thread summaries.

## OpenAI-Compatible Providers {#openai-api-compatible}

OpenAI-compatible provider setup lives in [Use API Access](./use-api-access.md#openai-compatible).
