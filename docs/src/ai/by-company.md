---
title: AI by Company - Zed
description: Find the right Zed setup path for OpenAI, ChatGPT, Codex, Claude, Gemini, Copilot, Cursor, OpenCode, Pi, Poolside, local models, and other AI tools.
---

# AI by Company

Use this page when you know the company, subscription, provider, agent, or CLI you want to use in Zed.

For detailed setup, follow the links in the `Setup` column. This page answers routing questions; it does not replace the setup pages.

## Zed {#zed}

Zed-hosted models and the Zed Pro/Business plans were removed from this fork.
Zed's own built-in AI surface is edit prediction, which you point at a local or
self-hosted model.

| Path                 | Support level     | What you get                      | Account / billing | Setup                                   |
| -------------------- | ----------------- | --------------------------------- | ----------------- | --------------------------------------- |
| Edit prediction      | Configured in Zed | Edit predictions while you type   | Local/self-hosted | [Edit Prediction](./edit-prediction.md) |

## OpenAI / ChatGPT / Codex {#openai-chatgpt-codex}

| Path                 | Support level     | What you get                                          | Account / billing     | Setup                                                                     |
| -------------------- | ----------------- | ----------------------------------------------------- | --------------------- | ------------------------------------------------------------------------- |
| OpenAI API           | Configured in Zed | OpenAI models through API access                      | OpenAI API billing    | [Use API Access](./use-api-access.md#openai)                              |
| Codex via ACP        | Hosted in Zed     | Codex in an External Agent thread                     | Owned by Codex/OpenAI | [External Agents](./external-agents.md#codex-cli)                         |
| Codex CLI            | Run in terminal   | Native Codex CLI experience in a Terminal Thread      | Owned by Codex/OpenAI | [Terminal Threads](./terminal-threads.md)                                 |

There is no ChatGPT subscription provider in this fork. ChatGPT Plus and Pro are
only usable through the Codex harness.

## Anthropic / Claude / Claude Code {#anthropic-claude}

| Path                 | Support level     | What you get                                       | Account / billing                       | Setup                                                |
| -------------------- | ----------------- | -------------------------------------------------- | --------------------------------------- | ---------------------------------------------------- |
| Claude Agent via ACP | Hosted in Zed     | Claude in an External Agent thread                 | Owned by Claude/Anthropic               | [External Agents](./external-agents.md#claude-agent) |
| Claude Code CLI      | Run in terminal   | Native Claude Code experience in a Terminal Thread | Claude subscription or Claude Code auth | [Terminal Threads](./terminal-threads.md)            |

The Anthropic model provider was removed from this fork, so Claude is only
available through the Claude Agent or Claude Code. Claude Pro and Max
subscriptions are separate from Anthropic API credits.

## Google / Gemini / Gemini CLI {#google-gemini}

| Path          | Support level                    | What you get                                      | Account / billing     | Setup                                                                                         |
| ------------- | -------------------------------- | ------------------------------------------------- | --------------------- | --------------------------------------------------------------------------------------------- |
| Gemini CLI    | Hosted in Zed or run in terminal | Gemini CLI as an External Agent or native CLI/TUI | Owned by Gemini CLI   | [External Agents](./external-agents.md#gemini-cli), [Terminal Threads](./terminal-threads.md) |

The Google AI model provider was removed from this fork; Gemini models are only
usable through the Gemini CLI harness.

## GitHub / Copilot {#github-copilot}

| Path                    | Support level     | What you get                                          | Account / billing           | Setup                                                                            |
| ----------------------- | ----------------- | ----------------------------------------------------- | --------------------------- | -------------------------------------------------------------------------------- |
| Copilot edit prediction | Removed           | Edit prediction provider removed, no longer available | n/a                         | [Edit Prediction](./edit-prediction.md#github-copilot)                           |
| Copilot External Agent  | Hosted in Zed     | Copilot in an External Agent thread, where available  | Owned by Copilot            | [External Agents](./external-agents.md#copilot)                                  |
| Copilot CLI             | Run in terminal   | Native CLI experience, where available                | Owned by Copilot            | [Terminal Threads](./terminal-threads.md)                                        |

GitHub Copilot is not a Zed model provider in this fork; it is only usable
through the Copilot agent or CLI.

## OpenCode {#opencode}

| Path                    | Support level   | What you get                                          | Account / billing | Setup                                            |
| ----------------------- | --------------- | ----------------------------------------------------- | ----------------- | ------------------------------------------------ |
| OpenCode External Agent | Hosted in Zed   | OpenCode in an External Agent thread, where available | Owned by OpenCode | [External Agents](./external-agents.md#opencode) |
| `opencode` CLI          | Run in terminal | Native OpenCode CLI experience                        | Owned by OpenCode | [Terminal Threads](./terminal-threads.md)        |

## Cursor {#cursor}

| Path                  | Support level   | What you get                                           | Account / billing           | Setup                                          |
| --------------------- | --------------- | ------------------------------------------------------ | --------------------------- | ---------------------------------------------- |
| Cursor External Agent | Hosted in Zed   | Cursor in an External Agent thread, where available    | Cursor account/subscription | [External Agents](./external-agents.md#cursor) |
| Cursor CLI/TUI        | Run in terminal | Native Cursor command-line experience, where available | Cursor account/subscription | [Terminal Threads](./terminal-threads.md)      |

Cursor subscriptions do not configure Zed's LLM provider settings. If you want to use a work Cursor subscription in Zed, use the Cursor External Agent or a Terminal Threads workflow where available.

## Pi Coding Agent {#pi}

| Path            | Support level   | What you get                                       | Account / billing | Setup                                      |
| --------------- | --------------- | -------------------------------------------------- | ----------------- | ------------------------------------------ |
| Pi Coding Agent | Hosted in Zed   | Pi in an External Agent thread, where available    | Owned by Pi       | [External Agents](./external-agents.md#pi) |
| Pi CLI/TUI      | Run in terminal | Native Pi command-line experience, where available | Owned by Pi       | [Terminal Threads](./terminal-threads.md)  |

Pi is an agent harness, not a Zed LLM subscription. Pi may support provider auth such as ChatGPT, Claude, or Copilot through its own setup flow.

## Poolside {#poolside}

| Path                    | Support level   | What you get                         | Account / billing               | Setup                                            |
| ----------------------- | --------------- | ------------------------------------ | ------------------------------- | ------------------------------------------------ |
| Poolside External Agent | Hosted in Zed   | Poolside in an External Agent thread | Poolside or configured provider | [External Agents](./external-agents.md#poolside) |
| `pool` CLI              | Run in terminal | Native Poolside Agent CLI experience | Poolside or configured provider | [Terminal Threads](./terminal-threads.md)        |

Install Poolside from the ACP Registry, configure Zed with the Poolside Agent CLI, or add Poolside as a Custom Agent. See [External Agents](./external-agents.md#poolside) for setup steps and platform-specific details.

## DeepSeek {#deepseek}

| Path         | Support level     | What you get                        | Account / billing                               | Setup                                          |
| ------------ | ----------------- | ----------------------------------- | ----------------------------------------------- | ---------------------------------------------- |
| DeepSeek API | Configured in Zed | DeepSeek models for Zed AI features | DeepSeek API credits, top-ups, or usage billing | [Use API Access](./use-api-access.md#deepseek) |

Paid DeepSeek usage is API access in Zed, not subscription sign-in.

## Gateways and Cloud Platforms {#gateways}

For gateway or cloud-platform model access, configure an [OpenAI-compatible gateway](./use-a-gateway.md#openai-compatible).

## Local Models {#local-models}

| Tool                              | Support level     | What you get                           | Account / billing | Setup                                                         |
| --------------------------------- | ----------------- | -------------------------------------- | ----------------- | ------------------------------------------------------------- |
| llama.cpp                         | Configured in Zed | Local models for Zed AI features       | Local/self-hosted | [Use a Local Model](./use-a-local-model.md#llama-cpp)         |
| LM Studio                         | Configured in Zed | Local models for Zed AI features       | Local/self-hosted | [Use a Local Model](./use-a-local-model.md#lm-studio)         |
| Ollama                            | Configured in Zed | Local models for Zed AI features       | Local/self-hosted | [Use a Local Model](./use-a-local-model.md#ollama)            |
| Local OpenAI-compatible server    | Configured in Zed | Local or self-hosted model endpoint    | Local/self-hosted | [Use a Local Model](./use-a-local-model.md#openai-compatible) |
| Local/self-hosted edit prediction | Configured in Zed | Edit predictions from a local provider | Local/self-hosted | [Edit Prediction](./edit-prediction.md)                       |

## Other API Providers {#other-api-providers}

For OpenAI-compatible endpoints that are not listed above, see [Use API Access](./use-api-access.md#openai-compatible).
