---
title: Use API Access - Zed
description: Configure provider API access, API keys, API credits, usage billing, and OpenAI-compatible endpoints for Zed AI features.
---

# Use API Access

Use API access when a provider gives you an API key, API credits, top-ups, or usage billing.

Paid API credits, usage billing, and top-ups are API access, even when you pay the provider directly. Use this path when the provider gives you an API key.

## Supported API Providers {#providers}

Zed supports these first-class API providers for model-backed Zed AI features:

- [Anthropic](#anthropic)
  - [Custom Anthropic Models](#anthropic-custom-models)
- [OpenAI API](#openai)
  - [Custom OpenAI Models](#openai-custom-models)
- [Google AI](#google-ai)
  - [Custom Google AI Models](#google-ai-custom-models)
- [DeepSeek](#deepseek)
  - [Custom DeepSeek Models](#deepseek-custom-models)
- [Anthropic-compatible endpoints](#anthropic-compatible)
- [OpenAI-compatible endpoints](#openai-compatible)

## What API Access Applies To {#support}

Use API access for the Zed Agent, Inline Assistant, Git commit generation,
thread summaries, and similar Zed-owned AI features.

External Agents and Terminal Threads usually configure model access in the
agent or CLI itself. See [Agents](./agents.md) for the difference between
agent paths and model access paths.

## API Keys and Environment Variables {#api-keys}

Most API-access providers can be configured on the **Settings → AI → LLM Providers** page with {#action agent::OpenSettings}. Keys saved through Zed are stored in the system keychain, not in `settings.json`.

Zed also reads provider-specific environment variables. Non-empty environment variables take precedence over keychain values. If a key comes from an environment variable, unset the variable and restart Zed to stop using it.

| Provider  | Environment variable                                  |
| --------- | ----------------------------------------------------- |
| Anthropic | `ANTHROPIC_API_KEY`                                   |
| OpenAI    | `OPENAI_API_KEY`                                      |
| Google AI | `GEMINI_API_KEY`, falling back to `GOOGLE_AI_API_KEY` |
| DeepSeek  | `DEEPSEEK_API_KEY`                                    |
| Ollama    | `OLLAMA_API_KEY`                                      |
| LM Studio | `LMSTUDIO_API_KEY`                                    |

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

`custom_headers` is supported by Anthropic, DeepSeek, Google AI, LM Studio, Ollama, OpenAI, and OpenAI-compatible providers.

Headers managed by Zed for each provider, such as `Authorization`, `Content-Type`, `Accept`, and provider-specific authentication headers, are ignored with a warning if you try to override them.

External Agents and Terminal Threads may run their own processes and use their own remote or local environment. See [External Agents](./external-agents.md) and [Terminal Threads](./terminal-threads.md).

## Provider Notes {#provider-notes}

### Anthropic {#anthropic}

Use Anthropic API access when you have an Anthropic API key or API credits. Claude Pro and Max subscriptions are separate; see [Use an Existing Subscription](./use-an-existing-subscription.md#claude).

1. Sign up for Anthropic and [create an API key](https://console.anthropic.com/settings/keys).
2. Make sure your Anthropic account has API credits.
3. Open Agent Settings with {#action agent::OpenSettings} and go to the Anthropic section.
4. Enter your Anthropic API key.

Zed also reads `ANTHROPIC_API_KEY` from the local Zed process environment.

#### Custom Anthropic Models {#anthropic-custom-models}

Add custom Anthropic models in settings when you need an alternate model ID,
display name, context window, output limit, tool override, or thinking mode.

```json [settings]
{
  "language_models": {
    "anthropic": {
      "available_models": [
        {
          "name": "claude-3-5-sonnet-20240620",
          "display_name": "Sonnet 2024-June",
          "max_tokens": 128000,
          "max_output_tokens": 2560,
          "tool_override": "some-model-that-supports-toolcalling"
        }
      ]
    }
  }
}
```

For Anthropic models that support extended thinking, add a `mode` configuration:

```json [settings]
{
  "language_models": {
    "anthropic": {
      "available_models": [
        {
          "name": "claude-sonnet-4-latest",
          "display_name": "claude-sonnet-4-thinking",
          "max_tokens": 200000,
          "mode": {
            "type": "thinking",
            "budget_tokens": 4096
          }
        }
      ]
    }
  }
}
```

### OpenAI API {#openai}

Use OpenAI API access when you have an OpenAI API key or API billing. ChatGPT Plus and Pro subscriptions use a different setup path; see [Use an Existing Subscription](./use-an-existing-subscription.md#chatgpt).

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

### Google AI {#google-ai}

Use Google AI API access when you have a Gemini API key.

1. Go to Google AI Studio and [create an API key](https://aistudio.google.com/app/apikey).
2. Open Agent Settings with {#action agent::OpenSettings} and go to the Google AI section.
3. Enter your Google AI API key.

Zed reads `GEMINI_API_KEY`, falling back to `GOOGLE_AI_API_KEY`, from the local Zed process environment.

#### Custom Google AI Models {#google-ai-custom-models}

Add custom Google AI models when you need a specific Gemini model version,
including experimental models, or a thinking-mode configuration.

```json [settings]
{
  "language_models": {
    "google": {
      "available_models": [
        {
          "name": "gemini-3.1-pro-preview",
          "display_name": "Gemini 3.1 Pro",
          "max_tokens": 1000000,
          "mode": {
            "type": "thinking",
            "budget_tokens": 24000
          }
        },
        {
          "name": "gemini-3-flash-preview",
          "display_name": "Gemini 3 Flash (Thinking)",
          "max_tokens": 1000000,
          "mode": {
            "type": "thinking",
            "budget_tokens": 24000
          }
        }
      ]
    }
  }
}
```

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

### Anthropic-Compatible Endpoints {#anthropic-compatible}

Use an Anthropic-compatible endpoint when a service implements Anthropic's [Messages API](https://docs.anthropic.com/en/api/messages) (`/v1/messages`) and gives you a custom base URL, model ID, and API key.

You can add a custom Anthropic-compatible provider from Agent Settings with {#action agent::OpenSettings}. Look for `Add Provider` in the LLM Providers section, choose `Anthropic`, and fill in the provider name, API URL, model ID, and context window.

You can also configure the provider in your settings file:

```json [settings]
{
  "language_models": {
    "anthropic_compatible": {
      "Some Provider": {
        "api_url": "https://api.someprovider.com",
        "custom_headers": {
          "X-Some-Header": "some-value"
        },
        "available_models": [
          {
            "name": "some-model",
            "display_name": "Some Model",
            "max_tokens": 200000,
            "max_output_tokens": 32000,
            "capabilities": {
              "tools": true,
              "images": false,
              "prompt_caching": false
            }
          }
        ]
      }
    }
  }
}
```

By default, Anthropic-compatible models inherit these capabilities:

- `tools`: `true`
- `images`: `false`
- `prompt_caching`: `false`

Enable `prompt_caching` to send explicit `cache_control` breakpoints for [prompt caching](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching); leave it disabled if the provider rejects requests containing them.

The optional `custom_headers` map adds extra headers to every request, which some providers require. Headers managed by Zed (such as `X-Api-Key` and `Anthropic-Version`) cannot be overridden.

Models also support the optional `default_temperature`, `extra_beta_headers` (sent as `anthropic-beta` headers), `mode`, and `tool_override` fields, which behave the same as in [Custom Anthropic Models](#anthropic-custom-models).

Enter the API key in the provider settings UI or set the generated environment variable (`<PROVIDER_NAME>_API_KEY`; in the example above, `SOME_PROVIDER_API_KEY`). Do not put API keys in `settings.json`.

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
