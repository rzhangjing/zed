use crate::merge_from::MergeFrom;
use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

use std::sync::Arc;

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AllLanguageModelSettingsContent {
    pub deepseek: Option<DeepseekSettingsContent>,
    #[serde(rename = "llama.cpp")]
    pub llama_cpp: Option<LlamaCppSettingsContent>,
    pub lmstudio: Option<LmStudioSettingsContent>,
    pub ollama: Option<OllamaSettingsContent>,
    pub openai: Option<OpenAiSettingsContent>,
    pub openai_compatible: Option<HashMap<Arc<str>, OpenAiCompatibleSettingsContent>>,
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

fn default_true() -> bool {
    true
}

