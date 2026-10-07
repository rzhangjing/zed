---
title: Authentication - Zed
description: "Configure API keys for the AI providers supported by this fork."
---

# Authentication

This fork does not have a Zed account, a sign-in flow, or a Zed-hosted model
service. No feature requires an account, and there is no network connection to
`zed.dev` or `collab.zed.dev`.

## AI Provider Keys {#ai-provider-keys}

Zed's AI features read credentials for the providers you configure:

- [OpenAI](./ai/use-api-access.md#openai)
- [DeepSeek](./ai/use-api-access.md#deepseek)
- [OpenAI-compatible endpoints](./ai/use-api-access.md#openai-compatible)
- [Ollama](./ai/use-a-local-model.md#ollama)
- [LM Studio](./ai/use-a-local-model.md#lm-studio)
- [llama.cpp](./ai/use-a-local-model.md#llama-cpp)

Keys entered on the **Settings → AI → LLM Providers** page ({#action agent::OpenSettings}) are stored in your operating system keychain, not in `settings.json`.

Zed also reads provider-specific environment variables:

| Provider  | Environment variable |
| --------- | -------------------- |
| OpenAI    | `OPENAI_API_KEY`     |
| DeepSeek  | `DEEPSEEK_API_KEY`   |
| Ollama    | `OLLAMA_API_KEY`     |
| LM Studio | `LMSTUDIO_API_KEY`   |

Environment variables take precedence over keychain values. If a key comes from an environment variable, unset the variable and restart Zed to stop using it.

OpenAI-compatible provider environment variables are generated from the configured provider ID as upper snake case plus `_API_KEY`. For example, provider ID `my-gateway` uses `MY_GATEWAY_API_KEY`.

## Where Credentials Live {#credentials}

Credentials are stored through your operating system's credential store (Keychain on macOS, Credential Manager on Windows, and Secret Service on Linux). They are never written to `settings.json`.

## See Also

- [Use API Access](./ai/use-api-access.md)
- [Use a Local Model](./ai/use-a-local-model.md)
- [AI Privacy](./ai/privacy-and-security.md)
