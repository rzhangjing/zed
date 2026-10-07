---
title: Use an Existing Subscription - Zed
description: Use ChatGPT, Claude, Copilot, OpenCode, Cursor, and other existing AI subscriptions in Zed.
---

# Use an Existing Subscription

Use this page when you already pay for an AI product and want to know how it fits into Zed.

Some subscriptions work as Zed model providers. Others are used through an External Agent or terminal CLI.

| Subscription                  | Zed AI features                                      | External Agent via ACP                | Terminal Thread                | Notes                                                            |
| ----------------------------- | ---------------------------------------------------- | ------------------------------------- | ------------------------------ | ---------------------------------------------------------------- |
| ChatGPT Plus / Pro            | Removed in this fork                                 | Codex where supported                 | Codex CLI                      | Use the Codex harness; it owns the auth                          |
| Claude Pro / Max              | No direct Zed LLM provider path                      | Claude Agent                          | Claude Code                    | Anthropic provider removed from this fork                        |
| GitHub Copilot                | No direct Zed LLM provider path                      | Copilot agent where available         | CLI where available            | Requires Copilot agent or CLI auth                               |
| OpenCode Zen / Go             | No direct Zed LLM provider path                      | OpenCode agent where available        | `opencode` CLI                 | Requires OpenCode agent or CLI; Zed does not sign in to OpenCode |
| Cursor subscription           | No Zed LLM provider path                             | Cursor External Agent where available | Cursor CLI/TUI where available | Use agent/CLI paths instead of Zed LLM provider settings         |

Zed Pro, Business, and Student plans, and the Zed-hosted models they unlocked,
were removed from this fork. So was the ChatGPT subscription provider: in this
fork Zed does not sign in to OpenAI either.

## ChatGPT Plus / Pro {#chatgpt}

ChatGPT Plus and Pro are only usable through the Codex harness (External Agent
or CLI), which owns its own auth. There is no ChatGPT subscription provider in
Zed in this fork.

OpenAI API access is separate. If you have OpenAI API credits or API billing, use [Use API Access](./use-api-access.md#openai).

## Claude Pro / Max {#claude}

Claude Pro and Max subscriptions are separate from Anthropic API credits. The
Anthropic provider was removed from this fork, so the only way to use Claude
subscription limits is Claude Agent or Claude Code.

## GitHub Copilot {#github-copilot}

GitHub Copilot was removed as a Zed model provider and as an [Edit Prediction](./edit-prediction.md) provider. Zed no longer signs in to GitHub Copilot for those features.

If you use a Copilot agent or CLI, that setup is owned by Copilot. See [External Agents](./external-agents.md) and [Terminal Threads](./terminal-threads.md).

## OpenCode Zen / Go {#opencode}

OpenCode Zen and Go are OpenCode's own subscription plans. Zed does not act as an OpenCode model provider; use the [OpenCode External Agent](./external-agents.md#opencode) or the `opencode` CLI in a [Terminal Thread](./terminal-threads.md) so OpenCode handles its own auth and model selection.

## Cursor {#cursor}

Cursor subscriptions do not configure Zed's LLM provider settings. Use a Cursor External Agent or Cursor CLI/TUI where available.

## Subscriptions Used Through Agent Harnesses {#agent-harnesses}

Some harnesses, CLIs, and External Agents can authenticate to ChatGPT, Claude, Copilot, or other subscriptions through their own flows. In those cases, Zed hosts the External Agent or Terminal Thread, but the harness owns auth and model behavior.

Pi Coding Agent is an example: Pi is a harness, not the subscription. Configure provider auth in Pi.

## DeepSeek {#deepseek}

DeepSeek paid usage, top-ups, and API billing are API access in Zed, not subscription sign-in. Use [Use API Access](./use-api-access.md#deepseek).
