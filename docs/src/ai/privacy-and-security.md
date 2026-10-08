---
title: AI Privacy - Zed
description: Understand how this fork handles AI prompts, code context, provider data boundaries, and privacy controls.
---

# AI Privacy

This page explains the privacy and trust boundaries for AI features, including
[Zed Agent](./zed-agent.md) and [Edit Prediction](./edit-prediction.md).

This fork does not route AI requests through Zed-hosted services. There are no
Zed-hosted models, and Zed has no model provider agreements to describe, because
every request goes from your machine to a provider you configured yourself. Your
prompts and code context are handled under that provider's terms.

## AI Request Paths {#ai-request-paths}

| Path                                                         | Who handles model requests                        | What to know                                                                                                                                                                                                                   | Details                                                                                           |
| ------------------------------------------------------------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| [Provider API keys](./use-api-access.md)                     | The configured provider                           | The provider handles requests under its own terms. Provider keys saved through Zed are stored in the system keychain, not in `settings.json`.                                                                                  | [Use API Access](./use-api-access.md)                                                             |
| [Existing subscriptions](./use-an-existing-subscription.md)  | The subscription or harness owner                 | In this fork subscriptions are owned by an External Agent or CLI, not by Zed.                                                                                                                                                   | [Use an Existing Subscription](./use-an-existing-subscription.md)                                 |
| [Gateways](./use-a-gateway.md)                               | The configured gateway and upstream providers     | The gateway and upstream providers handle requests under their own terms.                                                                                                                                                      | [Use a Gateway](./use-a-gateway.md)                                                               |
| [Local models](./use-a-local-model.md)                       | The local server or self-hosted endpoint          | The local server handles requests according to how you configured that server.                                                                                                                                                 | [Use a Local Model](./use-a-local-model.md)                                                       |
| [External Agents](./external-agents.md)                      | The External Agent and its configured providers   | The External Agent handles model requests under its own terms. Tool and MCP behavior depends on agent and ACP configuration.                                                                                                   | [External Agents](./external-agents.md)                                                           |
| [Terminal Threads](./terminal-threads.md)                    | The CLI or TUI running in the terminal            | The CLI or TUI owns its auth, model routing, tools, instructions, MCP configuration, and data handling.                                                                                                                        | [Terminal Threads](./terminal-threads.md)                                                         |
| [Edit Prediction](./edit-prediction.md)                      | The selected edit prediction provider             | Each keystroke can send local editing context to the selected provider, which in this fork is always a local or self-hosted endpoint you configure.                                                                            | [Edit Prediction](./edit-prediction.md), [Feedback and Training Data](./ai-improvement.md)        |
| [Agent tools](./tools.md), [MCP](./mcp.md), and integrations | Zed, configured MCP servers, and external systems | Tools can read, edit, search, run commands, fetch URLs, or call external systems depending on profile, MCP server, and tool permission settings.                                                                               | [Agent Profiles](./agent-profiles.md), [Tool Permissions](./tool-permissions.md), [MCP](./mcp.md) |
| Project trust and instructions                               | Zed and the trusted worktree                      | Project-local instructions and skills are loaded from trusted worktrees. External Agents and Terminal Threads may read their own instruction files.                                                                            | [Worktree Trust](../worktree-trust.md), [Skills](./skills.md), [Instructions](./instructions.md)  |

## Removed in This Fork {#removed}

- Zed-hosted models and the provider agreements that governed them
- Provider-designated retention for Zed-hosted models (for example Anthropic's
  Covered Models retention)
- Org-wide privacy enforcement for Zed Business
- Telemetry upload of usage metrics (see [Telemetry](../telemetry.md))

If you configure a provider directly, that provider's own retention and training
policies apply to your requests. Check the provider's documentation.

## AI Data Retained by Zed {#ai-data-retained-by-zed}

Nothing here calls home. See [Feedback and Training Data](./ai-improvement.md)
for what the opt-in feedback and training-data settings do.

## Controls and Related Privacy Docs {#controls-and-related-privacy-docs}

- [Telemetry](../telemetry.md): What is and is not collected.
- [AI Quick Start](./quick-start.md#turn-ai-off): How to turn AI off.
- [Privacy Policy](https://zed.dev/privacy-policy): Zed's privacy policy.
- [Subprocessors](https://zed.dev/subprocessors): Zed's subprocessors.
- [Terms of Service](https://zed.dev/terms): Zed's terms.
