use std::sync::Arc;

use collections::HashMap;
use settings::RegisterSetting;

use crate::provider::{
    deepseek::DeepSeekSettings,
    llama_cpp::LlamaCppSettings, lmstudio::LmStudioSettings, ollama::OllamaSettings,
    open_ai::OpenAiSettings, open_ai_compatible::OpenAiCompatibleSettings, resolve_custom_headers,
};

#[derive(Debug, RegisterSetting)]
pub struct AllLanguageModelSettings {
    pub deepseek: DeepSeekSettings,
    pub llama_cpp: LlamaCppSettings,
    pub lmstudio: LmStudioSettings,
    pub ollama: OllamaSettings,
    pub openai: OpenAiSettings,
    pub openai_compatible: HashMap<Arc<str>, OpenAiCompatibleSettings>,
}

fn custom_headers_from(
    provider_name: &str,
    raw: Option<HashMap<String, String>>,
    reserved: &[&str],
) -> http_client::CustomHeaders {
    raw.as_ref()
        .filter(|map| !map.is_empty())
        .map(|map| resolve_custom_headers(provider_name, map, reserved))
        .unwrap_or_default()
}

impl settings::Settings for AllLanguageModelSettings {
    const PRESERVED_KEYS: Option<&'static [&'static str]> = Some(&["version"]);

    fn from_settings(content: &settings::SettingsContent) -> Self {
        let language_models = content.language_models.clone().unwrap();
        let deepseek = language_models.deepseek.unwrap();
        let llama_cpp = language_models.llama_cpp.unwrap();
        let lmstudio = language_models.lmstudio.unwrap();
        let ollama = language_models.ollama.unwrap();
        let openai = language_models.openai.unwrap();
        let openai_compatible = language_models.openai_compatible.unwrap();
        Self {
            deepseek: DeepSeekSettings {
                api_url: deepseek.api_url.unwrap(),
                available_models: deepseek.available_models.unwrap_or_default(),
                custom_headers: custom_headers_from("DeepSeek", deepseek.custom_headers, &[]),
            },
            llama_cpp: LlamaCppSettings {
                api_url: llama_cpp.api_url.unwrap(),
                auto_discover: llama_cpp.auto_discover.unwrap_or(true),
                available_models: llama_cpp.available_models.unwrap_or_default(),
                context_window: llama_cpp.context_window,
                custom_headers: custom_headers_from("llama.cpp", llama_cpp.custom_headers, &[]),
            },
            lmstudio: LmStudioSettings {
                api_url: lmstudio.api_url.unwrap(),
                available_models: lmstudio.available_models.unwrap_or_default(),
                custom_headers: custom_headers_from("LM Studio", lmstudio.custom_headers, &[]),
            },
            ollama: OllamaSettings {
                api_url: ollama.api_url.unwrap(),
                auto_discover: ollama.auto_discover.unwrap_or(true),
                available_models: ollama.available_models.unwrap_or_default(),
                context_window: ollama.context_window,
                custom_headers: custom_headers_from("Ollama", ollama.custom_headers, &[]),
            },
            openai: OpenAiSettings {
                api_url: openai.api_url.unwrap(),
                available_models: openai.available_models.unwrap_or_default(),
                custom_headers: custom_headers_from("OpenAI", openai.custom_headers, &[]),
            },
            openai_compatible: openai_compatible
                .into_iter()
                .map(|(key, value)| {
                    let provider_label = format!("OpenAI Compatible ({key})");
                    (
                        key,
                        OpenAiCompatibleSettings {
                            api_url: value.api_url,
                            available_models: value.available_models,
                            custom_headers: custom_headers_from(
                                &provider_label,
                                value.custom_headers,
                                &[],
                            ),
                        },
                    )
                })
                .collect(),
        }
    }
}
