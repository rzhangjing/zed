use crate::merge_from::MergeFrom;
use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

use std::sync::Arc;

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AllLanguageModelSettingsContent {
    pub anthropic: Option<AnthropicSettingsContent>,
    pub anthropic_compatible: Option<HashMap<Arc<str>, AnthropicCompatibleSettingsContent>>,
    pub deepseek: Option<DeepseekSettingsContent>,
    pub google: Option<GoogleSettingsContent>,
    #[serde(rename = "llama.cpp")]
    pub llama_cpp: Option<LlamaCppSettingsContent>,
    pub lmstudio: Option<LmStudioSettingsContent>,
    pub ollama: Option<OllamaSettingsContent>,
    pub openai: Option<OpenAiSettingsContent>,
    pub openai_compatible: Option<HashMap<Arc<str>, OpenAiCompatibleSettingsContent>>,
    #[serde(rename = "zed.dev")]
    pub zed_dot_dev: Option<ZedDotDevSettingsContent>,
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AnthropicSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<AnthropicAvailableModel>>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AnthropicCompatibleSettingsContent {
    pub api_url: String,
    pub available_models: Vec<AnthropicCompatibleAvailableModel>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct AnthropicCompatibleAvailableModel {
    /// The model's name in the provider's API. e.g. claude-3-5-sonnet-latest
    pub name: String,
    /// The model's name in Zed's UI, such as in the model selector dropdown menu in the assistant panel.
    pub display_name: Option<String>,
    /// The model's context window size.
    pub max_tokens: u64,
    /// A model `name` to substitute when calling tools, in case the primary model doesn't support tool calling.
    pub tool_override: Option<String>,
    pub max_output_tokens: Option<u64>,
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub extra_beta_headers: Vec<String>,
    /// The model's mode (e.g. thinking)
    pub mode: Option<ModelMode>,
    #[serde(default)]
    pub capabilities: AnthropicCompatibleModelCapabilities,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct AnthropicCompatibleModelCapabilities {
    pub tools: bool,
    pub images: bool,
    /// Whether to send explicit `cache_control` breakpoints for prompt caching.
    /// Leave disabled if the provider rejects requests containing them.
    pub prompt_caching: bool,
}

impl Default for AnthropicCompatibleModelCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            images: false,
            prompt_caching: false,
        }
    }
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct AnthropicAvailableModel {
    /// The model's name in the Anthropic API. e.g. claude-3-5-sonnet-latest, claude-3-opus-20240229, etc
    pub name: String,
    /// The model's name in Zed's UI, such as in the model selector dropdown menu in the agent panel.
    pub display_name: Option<String>,
    /// The model's context window size.
    pub max_tokens: u64,
    /// A model `name` to substitute when calling tools, in case the primary model doesn't support tool calling.
    pub tool_override: Option<String>,
    /// Configuration of Anthropic's caching API.
    pub cache_configuration: Option<LanguageModelCacheConfiguration>,
    pub max_output_tokens: Option<u64>,
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub extra_beta_headers: Vec<String>,
    /// Whether Anthropic's fast mode is available for this model.
    pub supports_fast_mode: Option<bool>,
    /// The model's mode (e.g. thinking)
    pub mode: Option<ModelMode>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OllamaSettingsContent {
    pub api_url: Option<String>,
    pub auto_discover: Option<bool>,
    pub available_models: Option<Vec<OllamaAvailableModel>>,
    pub context_window: Option<u64>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OllamaAvailableModel {
    /// The model name in the Ollama API (e.g. "llama3.2:latest")
    pub name: String,
    /// The model's name in Zed's UI, such as in the model selector dropdown menu in the agent panel.
    pub display_name: Option<String>,
    /// The Context Length parameter to the model (aka num_ctx or n_ctx)
    pub max_tokens: u64,
    /// The number of seconds to keep the connection open after the last request
    pub keep_alive: Option<KeepAlive>,
    /// Whether the model supports tools
    pub supports_tools: Option<bool>,
    /// Whether the model supports vision
    pub supports_images: Option<bool>,
    /// Whether to enable think mode
    pub supports_thinking: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, JsonSchema, MergeFrom)]
#[serde(untagged)]
pub enum KeepAlive {
    /// Keep model alive for N seconds
    Seconds(isize),
    /// Keep model alive for a fixed duration. Accepts durations like "5m", "10m", "1h", "1d", etc.
    Duration(String),
}

impl KeepAlive {
    /// Keep model alive until a new model is loaded or until Ollama shuts down
    pub fn indefinite() -> Self {
        Self::Seconds(-1)
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::indefinite()
    }
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct LmStudioSettingsContent {
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub available_models: Option<Vec<LmStudioAvailableModel>>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct LmStudioAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub supports_tool_calls: bool,
    pub supports_images: bool,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct LlamaCppSettingsContent {
    pub api_url: Option<String>,
    /// Whether to automatically discover models served by the llama.cpp server.
    /// Defaults to true.
    pub auto_discover: Option<bool>,
    pub available_models: Option<Vec<LlamaCppAvailableModel>>,
    /// Overrides the context length reported for every llama.cpp model.
    pub context_window: Option<u64>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct LlamaCppAvailableModel {
    /// The model id reported by the llama.cpp server (its `--alias` or the model file path).
    pub name: String,
    /// The model's name in Zed's UI, such as in the model selector dropdown menu in the agent panel.
    pub display_name: Option<String>,
    /// The Context Length parameter to the model (aka n_ctx).
    pub max_tokens: u64,
    /// Whether the model supports tools.
    pub supports_tools: Option<bool>,
    /// Whether the model supports vision.
    pub supports_images: Option<bool>,
    /// Whether the model emits reasoning/thinking content.
    pub supports_thinking: Option<bool>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct DeepseekSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<DeepseekAvailableModel>>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct DeepseekAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OpenAiSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<OpenAiAvailableModel>>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub reasoning_effort: Option<OpenAiReasoningEffort>,
    #[serde(default)]
    pub capabilities: OpenAiModelCapabilities,
}

pub use language_model_core::ReasoningEffort as OpenAiReasoningEffort;

impl MergeFrom for OpenAiReasoningEffort {
    fn merge_from(&mut self, other: &Self) {
        *self = *other;
    }
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleSettingsContent {
    pub api_url: String,
    pub available_models: Vec<OpenAiCompatibleAvailableModel>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiModelCapabilities {
    #[serde(default = "default_true")]
    pub chat_completions: bool,
    #[serde(default = "default_true")]
    pub images: bool,
}

impl Default for OpenAiModelCapabilities {
    fn default() -> Self {
        Self {
            chat_completions: default_true(),
            images: default_true(),
        }
    }
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub reasoning_effort: Option<OpenAiReasoningEffort>,
    #[serde(default)]
    pub capabilities: OpenAiCompatibleModelCapabilities,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleModelCapabilities {
    pub tools: bool,
    pub images: bool,
    pub parallel_tool_calls: bool,
    pub prompt_cache_key: bool,
    #[serde(default = "default_true")]
    pub chat_completions: bool,
    #[serde(default)]
    pub interleaved_reasoning: bool,
    #[serde(default)]
    pub max_tokens_parameter: bool,
}

impl Default for OpenAiCompatibleModelCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            images: false,
            parallel_tool_calls: false,
            prompt_cache_key: false,
            chat_completions: default_true(),
            interleaved_reasoning: false,
            max_tokens_parameter: false,
        }
    }
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct GoogleSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<GoogleAvailableModel>>,
    pub custom_headers: Option<HashMap<String, String>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct GoogleAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub mode: Option<ModelMode>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct ZedDotDevSettingsContent {
    pub available_models: Option<Vec<ZedDotDevAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct ZedDotDevAvailableModel {
    /// The provider of the language model.
    pub provider: ZedDotDevAvailableProvider,
    /// The model's name in the provider's API. e.g. claude-3-5-sonnet-20240620
    pub name: String,
    /// The name displayed in the UI, such as in the agent panel model dropdown menu.
    pub display_name: Option<String>,
    /// The size of the context window, indicating the maximum number of tokens the model can process.
    pub max_tokens: usize,
    /// The maximum number of output tokens allowed by the model.
    pub max_output_tokens: Option<u64>,
    /// The maximum number of completion tokens allowed by the model (o1-* only)
    pub max_completion_tokens: Option<u64>,
    /// Override this model with a different Anthropic model for tool calls.
    pub tool_override: Option<String>,
    /// Indicates whether this custom model supports caching.
    pub cache_configuration: Option<LanguageModelCacheConfiguration>,
    /// The default temperature to use for this model.
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    /// Any extra beta headers to provide when using the model.
    #[serde(default)]
    pub extra_beta_headers: Vec<String>,
    /// The model's mode (e.g. thinking)
    pub mode: Option<ModelMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "lowercase")]
pub enum ZedDotDevAvailableProvider {
    Anthropic,
    OpenAi,
    Google,
}

fn default_true() -> bool {
    true
}

/// Configuration for caching language model messages.
#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct LanguageModelCacheConfiguration {
    pub max_cache_anchors: usize,
    pub should_speculate: bool,
    pub min_total_token: u64,
}

pub use language_model_core::ModelMode;

impl MergeFrom for ModelMode {
    fn merge_from(&mut self, other: &Self) {
        *self = *other;
    }
}
