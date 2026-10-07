---
title: Use API Access - Zed
description: Configure provider API access, API keys, API credits, usage billing, and OpenAI-compatible endpoints for Zed AI features.
---

# Use API Access

Use API access when a provider gives you an API key, API credits, top-ups, or usage billing.

Paid API credits, usage billing, and top-ups are API access, even when you pay the provider directly. Use this path when the provider gives you an API key.

## Supported API Providers {#providers}

Zed supports these first-class API providers for model-backed Zed AI features:

- [OpenAI API](#openai)
  - [Custom OpenAI Models](#openai-custom-models)
- [DeepSeek](#deepseek)
  - [Custom DeepSeek Models](#deepseek-custom-models)
- [OpenAI-compatible endpoints](#openai-compatible)

Anthropic, Google AI, and Anthropic-compatible endpoints were removed from this
fork. Claude and Gemini are still usable through
[External Agents](./external-agents.md) and
[Terminal Threads](./terminal-threads.md).

## What API Access Applies To {#support}

Use API access for the Zed Agent, Git commit generation,
thread summaries, and similar Zed-owned AI features.

External Agents and Terminal Threads usually configure model access in the
agent or CLI itself. See [Agents](./agents.md) for the difference between
agent paths and model access paths.

## API Keys and Environment Variables {#api-keys}

Most API-access providers can be configured on the **Settings → AI → LLM Providers** page with {#action agent::OpenSettings}. Keys saved through Zed are stored in the system keychain, not in `settings.json`.

Zed also reads provider-specific environment variables. Non-empty environment variables take precedence over keychain values. If a key comes from an environment variable, unset the variable and restart Zed to stop using it.

| Provider  | Environment variable |
| --------- | -------------------- |
| OpenAI    | `OPENAI_API_KEY`     |
| DeepSeek  | `DEEPSEEK_API_KEY`   |
| Ollama    | `OLLAMA_API_KEY`     |
| LM Studio | `LMSTUDIO_API_KEY`   |

OpenAI-compatible provider environment variables are generated from the configured provider ID as upper snake case plus `_API_KEY`. For example, provider ID `my-gateway` uses `MY_GATEWAY_API_KEY`.

## Custom Headers {#custom-headers}

You can attach extra HTTP headers to every request Zed makes to supported HTTP-based providers. This is useful in corporate environments or for observability tooling.

Configure them with `language_models.<provider>.custom_headers`:

```json [settings]
{
  "language_models": {
    "openai": {
      "custom_headers": {
        "Fancy-Auth": "Bearer <your-fancy-key>",
        "X-My-Tag": "zed"
      }
    }
  }
}
```

`custom_headers` is supported by DeepSeek, LM Studio, Ollama, OpenAI, and OpenAI-compatible providers.

Headers managed by Zed for each provider, such as `Authorization`, `Content-Type`, `Accept`, and provider-specific authentication headers, are ignored with a warning if you try to override them.

External Agents and Terminal Threads may run their own processes and use their own remote or local environment. See [External Agents](./external-agents.md) and [Terminal Threads](./terminal-threads.md).

## Provider Notes {#provider-notes}

### OpenAI API {#openai}

Use OpenAI API access when you have an OpenAI API key or API billing. This fork has no ChatGPT subscription sign-in; use the Codex harness if you want subscription-backed behavior.

1. Visit the OpenAI platform and [create an API key](https://platform.openai.com/account/api-keys).
2. Make sure your OpenAI account has credits or billing enabled.
3. Open Agent Settings with {#action agent::OpenSettings} and go to the OpenAI section.
4. Enter your OpenAI API key.

Zed also reads `OPENAI_API_KEY` from the local Zed process environment.

#### Custom OpenAI Models {#openai-custom-models}

Add custom OpenAI models in your settings file when you need alternate model IDs, preview releases, or custom request parameters.

```json [settings]
{
  "language_models": {
    "openai": {
      "available_models": [
        {
          "name": "gpt-5.2",
          "display_name": "gpt-5.2 high",
          "reasoning_effort": "high",
          "max_tokens": 272000,
          "max_completion_tokens": 20000
        }
      ]
    }
  }
}
```

You must provide the model's context window in `max_tokens`. For reasoning-focused models, set `max_completion_tokens` to avoid high reasoning-token costs.

### DeepSeek {#deepseek}

Use DeepSeek API access when you have paid API usage, top-ups, or an API key. In Zed, DeepSeek is API access, not subscription sign-in.

1. Visit the DeepSeek platform and [create an API key](https://platform.deepseek.com/api_keys).
2. Open Agent Settings with {#action agent::OpenSettings} and go to the DeepSeek section.
3. Enter your DeepSeek API key.

Zed also reads `DEEPSEEK_API_KEY` from the local Zed process environment.

#### Custom DeepSeek Models {#deepseek-custom-models}

Add custom DeepSeek models when you need alternate model IDs, custom token
limits, or a custom endpoint.

```json [settings]
{
  "language_models": {
    "deepseek": {
      "api_url": "https://api.deepseek.com/v1",
      "available_models": [
        {
          "name": "deepseek-v4-flash",
          "display_name": "DeepSeek V4 Flash",
          "max_tokens": 1000000,
          "max_output_tokens": 384000
        },
        {
          "name": "deepseek-v4-pro",
          "display_name": "DeepSeek V4 Pro",
          "max_tokens": 1000000,
          "max_output_tokens": 384000
        }
      ]
    }
  }
}
```

### OpenAI-Compatible Endpoints {#openai-compatible}

Use an OpenAI-compatible endpoint when you have a custom base URL, model ID, and API key.

You can add a custom OpenAI-compatible provider from Agent Settings with {#action agent::OpenSettings}. Look for `Add Provider` in the LLM Providers section and fill in the provider name, API URL, model ID, and context window.

You can also configure the provider in your settings file:

```json [settings]
{
  "language_models": {
    "openai_compatible": {
      "my-provider": {
        "api_url": "https://example.com/v1",
        "available_models": [
          {
            "name": "my-model",
            "display_name": "My Model",
            "max_tokens": 128000
          }
        ]
      }
    }
  }
}
```

By default, OpenAI-compatible models inherit these capabilities:

- `tools`: `true`
- `images`: `false`
- `parallel_tool_calls`: `false`
- `prompt_cache_key`: `false`
- `chat_completions`: `true`
- `interleaved_reasoning`: `false`
- `max_tokens_parameter`: `false`

If a model only works with the Responses API, set `capabilities.chat_completions` to `false`. Zed will use the Responses endpoint for that model.

For reasoning models (e.g. GPT-5), set `reasoning_effort` to the non-`none` effort level your endpoint supports. This enables thinking in the agent panel and tells Zed which effort to send when thinking is enabled. The provider settings UI can configure this when adding an OpenAI-compatible provider. Zed sends OpenAI-style `reasoning_effort` on chat-completions requests.

If the model requires the Responses API for reasoning state, set `capabilities.chat_completions` to `false`:

```json [settings]
{
  "language_models": {
    "openai_compatible": {
      "my-provider": {
        "api_url": "https://example.com/v1",
        "available_models": [
          {
            "name": "gpt-5",
            "max_tokens": 272000,
            "reasoning_effort": "high",
            "capabilities": {
              "tools": true,
              "images": false,
              "parallel_tool_calls": false,
              "prompt_cache_key": false,
              "chat_completions": false,
              "interleaved_reasoning": false,
              "max_tokens_parameter": false
            }
          }
        ]
      }
    }
  }
}
```

Valid settings values are `"none"`, `"minimal"`, `"low"`, `"medium"`, `"high"`, `"xhigh"`, and `"max"`. Use `"none"` in `settings.json` when you need to force reasoning off for an endpoint; the provider setup UI exposes the non-`none` values for thinking-capable models. For chat-completions endpoints that should receive prior thinking back in a dedicated `reasoning_content` field, also set `capabilities.interleaved_reasoning` to `true`. If the endpoint expects the output-token limit as `max_tokens` instead of `max_completion_tokens`, set `capabilities.max_tokens_parameter` to `true`.

For example, a chat-completions endpoint with the maximum OpenAI-style reasoning effort, streamed thinking, and `max_tokens` output limits can be configured as:

```json [settings]
{
  "language_models": {
    "openai_compatible": {
      "my-reasoning-provider": {
        "api_url": "https://example.com/v1",
        "available_models": [
          {
            "name": "reasoning-model",
            "max_tokens": 1000000,
            "max_output_tokens": 128000,
            "reasoning_effort": "max",
            "capabilities": {
              "tools": true,
              "images": false,
              "parallel_tool_calls": false,
              "prompt_cache_key": false,
              "chat_completions": true,
              "interleaved_reasoning": true,
              "max_tokens_parameter": true
            }
          }
        ]
      }
    }
  }
}
```

Enter the API key in the provider settings UI or set the generated environment variable. Do not put API keys in `settings.json`.
