use anyhow::Result;
use client::UserStore;
use edit_prediction::EditPredictionStore;
use edit_prediction_types::EditPredictionDelegateHandle;
use editor::{
    Editor, MultiBufferOffset, SelectionEffects, actions::ShowEditPrediction, scroll::Autoscroll,
};
use fs::Fs;
use gpui::{
    Action, Anchor, App, AsyncWindowContext, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Subscription, TaskExt, WeakEntity, actions, div,
};
use indoc::indoc;
use language::{
    EditPredictionsMode, File, Language,
    language_settings::{
        AllLanguageSettings, EditPredictionProvider, LanguageSettings, all_language_settings,
    },
};
use project::DisableAiSettings;
use regex::Regex;
use settings::{Settings, SettingsStore, update_settings_file};
use std::{
    sync::{Arc, LazyLock},
};
use ui::{
    ContextMenu, ContextMenuEntry, DocumentationSide, IconButton, IconButtonShape,
    Indicator, PopoverMenu, PopoverMenuHandle, Tooltip, prelude::*,
};
use util::ResultExt as _;

use workspace::{
    HideStatusItem, StatusItemView, Workspace, create_and_open_local_file, item::ItemHandle,
};
use zed_actions::OpenSettingsAt;

actions!(
    edit_prediction,
    [
        /// Toggles the edit prediction menu.
        ToggleMenu
    ]
);

const PRIVACY_DOCS: &str = "https://zed.dev/docs/ai/privacy-and-security";

pub struct EditPredictionButton {
    editor_subscription: Option<(Subscription, usize)>,
    editor_enabled: Option<bool>,
    editor_show_predictions: bool,
    editor_focus_handle: Option<FocusHandle>,
    language: Option<Arc<Language>>,
    file: Option<Arc<dyn File>>,
    edit_prediction_provider: Option<Arc<dyn EditPredictionDelegateHandle>>,
    fs: Arc<dyn Fs>,
    popover_menu_handle: PopoverMenuHandle<ContextMenu>,
}

impl Render for EditPredictionButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Return empty div if AI is disabled
        if DisableAiSettings::get_global(cx).disable_ai {
            return div().hidden();
        }

        let language_settings = all_language_settings(None, cx);

        match language_settings.edit_predictions.provider {
            EditPredictionProvider::OpenAiCompatibleApi => {
                let enabled = self.editor_enabled.unwrap_or(true);
                let this = cx.weak_entity();

                div().child(
                    PopoverMenu::new("openai-compatible-api")
                        .menu(move |window, cx| {
                            this.update(cx, |this, cx| {
                                this.build_edit_prediction_context_menu(
                                    EditPredictionProvider::OpenAiCompatibleApi,
                                    window,
                                    cx,
                                )
                            })
                            .ok()
                        })
                        .anchor(Anchor::BottomRight)
                        .trigger(
                            IconButton::new("openai-compatible-api-icon", IconName::AiOpenAiCompat)
                                .shape(IconButtonShape::Square)
                                .tab_index(0isize)
                                .aria_label("Edit Prediction")
                                .when(!enabled, |this| {
                                    this.indicator(Indicator::dot().color(Color::Ignored))
                                        .indicator_border_color(Some(
                                            cx.theme().colors().status_bar_background,
                                        ))
                                }),
                        )
                        .with_handle(self.popover_menu_handle.clone()),
                )
            }
            EditPredictionProvider::Ollama => {
                let enabled = self.editor_enabled.unwrap_or(true);
                let this = cx.weak_entity();

                div().child(
                    PopoverMenu::new("ollama")
                        .menu(move |window, cx| {
                            this.update(cx, |this, cx| {
                                this.build_edit_prediction_context_menu(
                                    EditPredictionProvider::Ollama,
                                    window,
                                    cx,
                                )
                            })
                            .ok()
                        })
                        .anchor(Anchor::BottomRight)
                        .trigger_with_tooltip(
                            IconButton::new("ollama-icon", IconName::AiOllama)
                                .shape(IconButtonShape::Square)
                                .tab_index(0isize)
                                .aria_label("Edit Prediction")
                                .when(!enabled, |this| {
                                    this.indicator(Indicator::dot().color(Color::Ignored))
                                        .indicator_border_color(Some(
                                            cx.theme().colors().status_bar_background,
                                        ))
                                }),
                            move |_window, cx| {
                                let settings = all_language_settings(None, cx);
                                let tooltip_meta = match settings.edit_predictions.ollama.as_ref() {
                                    Some(settings) if !settings.model.trim().is_empty() => {
                                        format!("Powered by Ollama ({})", settings.model)
                                    }
                                    _ => {
                                        "Ollama model not configured — configure a model before use"
                                            .to_string()
                                    }
                                };

                                Tooltip::with_meta(
                                    "Edit Prediction",
                                    Some(&ToggleMenu),
                                    tooltip_meta,
                                    cx,
                                )
                            },
                        )
                        .with_handle(self.popover_menu_handle.clone()),
                )
            }
            EditPredictionProvider::None => div().hidden(),
        }
    }
}

impl EditPredictionButton {
    pub fn new(
        fs: Arc<dyn Fs>,
        _user_store: Entity<UserStore>,
        popover_menu_handle: PopoverMenuHandle<ContextMenu>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_global::<SettingsStore>(move |_, cx| cx.notify())
            .detach();

        cx.observe_global::<EditPredictionStore>(move |_, cx| cx.notify())
            .detach();

        let open_ai_compatible_api_token_task =
            edit_prediction::open_ai_compatible::load_open_ai_compatible_api_token(cx);

        cx.spawn(async move |this, cx| {
            _ = open_ai_compatible_api_token_task.await;
            this.update(cx, |_, cx| {
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            editor_subscription: None,
            editor_enabled: None,
            editor_show_predictions: true,
            editor_focus_handle: None,
            language: None,
            file: None,
            edit_prediction_provider: None,
            popover_menu_handle,
            fs,
        }
    }

    fn add_provider_switching_section(
        &self,
        mut menu: ContextMenu,
        current_provider: EditPredictionProvider,
        cx: &mut App,
    ) -> ContextMenu {
        let available_providers = get_available_providers(cx);

        let providers: Vec<_> = available_providers
            .into_iter()
            .filter(|p| *p != EditPredictionProvider::None)
            .collect();

        if !providers.is_empty() {
            menu = menu.separator().header("Providers");

            for provider in providers {
                let Some(name) = provider.display_name() else {
                    continue;
                };
                let is_current = provider == current_provider;
                let fs = self.fs.clone();

                menu = menu.item(
                    ContextMenuEntry::new(name)
                        .toggleable(IconPosition::Start, is_current)
                        .handler(move |_, cx| {
                            set_completion_provider(fs.clone(), cx, provider);
                        }),
                )
            }
        }

        menu
    }

    fn add_configure_providers_item(&self, menu: ContextMenu) -> ContextMenu {
        menu.separator().item(
            ContextMenuEntry::new("Configure Providers")
                .icon(IconName::Settings)
                .icon_position(IconPosition::Start)
                .icon_color(Color::Muted)
                .handler(move |window, cx| {
                    telemetry::event!(
                        "Edit Prediction Menu Action",
                        action = "configure_providers",
                    );
                    window.dispatch_action(
                        OpenSettingsAt {
                            path: "edit_predictions.providers".to_string(),
                            target: None,
                        }
                        .boxed_clone(),
                        cx,
                    );
                }),
        )
    }

    pub fn build_language_settings_menu(
        &self,
        mut menu: ContextMenu,
        _window: &Window,
        cx: &mut App,
    ) -> ContextMenu {
        let fs = self.fs.clone();
        menu = menu.header("Show Edit Predictions For");

        let language_state = self.language.as_ref().map(|language| {
            (
                language.clone(),
                LanguageSettings::resolve(None, Some(&language.name()), cx).show_edit_predictions,
            )
        });

        if let Some(editor_focus_handle) = self.editor_focus_handle.clone() {
            let entry = ContextMenuEntry::new("This Buffer")
                .toggleable(IconPosition::Start, self.editor_show_predictions)
                .action(Box::new(editor::actions::ToggleEditPrediction))
                .handler(move |window, cx| {
                    editor_focus_handle.dispatch_action(
                        &editor::actions::ToggleEditPrediction,
                        window,
                        cx,
                    );
                });

            match language_state.clone() {
                Some((language, false)) => {
                    menu = menu.item(entry.disabled(true).documentation_aside(
                        DocumentationSide::Left,
                        move |_cx| {
                            Label::new(format!(
                                "Edit predictions are disabled for {}",
                                language.name()
                            ))
                            .into_any_element()
                        },
                    ));
                }
                Some(_) | None => menu = menu.item(entry),
            }
        }

        if let Some((language, language_enabled)) = language_state {
            let fs = fs.clone();
            let language_name = language.name();

            menu = menu.toggleable_entry(
                language_name.clone(),
                language_enabled,
                IconPosition::Start,
                None,
                move |_, cx| {
                    telemetry::event!(
                        "Edit Prediction Setting Changed",
                        setting = "language",
                        language = language_name.to_string(),
                        enabled = !language_enabled,
                    );
                    toggle_show_edit_predictions_for_language(language.clone(), fs.clone(), cx)
                },
            );
        }

        let settings = AllLanguageSettings::get_global(cx);

        let globally_enabled = settings.show_edit_predictions(None, cx);
        let entry = ContextMenuEntry::new("All Files")
            .toggleable(IconPosition::Start, globally_enabled)
            .action(workspace::ToggleEditPrediction.boxed_clone())
            .handler(|window, cx| {
                window.dispatch_action(workspace::ToggleEditPrediction.boxed_clone(), cx)
            });
        menu = menu.item(entry);

        let current_mode = settings.edit_predictions_mode();
        let subtle_mode = matches!(current_mode, EditPredictionsMode::Subtle);
        let eager_mode = matches!(current_mode, EditPredictionsMode::Eager);

        menu = menu
                .separator()
                .header("Display Modes")
                .item(
                    ContextMenuEntry::new("Eager")
                        .toggleable(IconPosition::Start, eager_mode)
                        .documentation_aside(DocumentationSide::Left, move |_| {
                            Label::new("Display predictions inline when there are no language server completions available.").into_any_element()
                        })
                        .handler({
                            let fs = fs.clone();
                            move |_, cx| {
                                telemetry::event!(
                                    "Edit Prediction Setting Changed",
                                    setting = "mode",
                                    value = "eager",
                                );
                                toggle_edit_prediction_mode(fs.clone(), EditPredictionsMode::Eager, cx)
                            }
                        }),
                )
                .item(
                    ContextMenuEntry::new("Subtle")
                        .toggleable(IconPosition::Start, subtle_mode)
                        .documentation_aside(DocumentationSide::Left, move |_| {
                            Label::new(concat!(
                                "Display predictions inline only when holding a modifier key (",
                                ui::alt_key_name!(),
                                " by default)."
                            ))
                            .into_any_element()
                        })
                        .handler({
                            let fs = fs.clone();
                            move |_, cx| {
                                telemetry::event!(
                                    "Edit Prediction Setting Changed",
                                    setting = "mode",
                                    value = "subtle",
                                );
                                toggle_edit_prediction_mode(fs.clone(), EditPredictionsMode::Subtle, cx)
                            }
                        }),
                );

        menu = menu.separator().header("Privacy");

        menu = menu.item(
            ContextMenuEntry::new("Configure Excluded Files")
                .icon(IconName::Lock)
                .icon_color(Color::Muted)
                .documentation_aside(DocumentationSide::Left, |_| {
                    Label::new(indoc!{"
                        Open your settings to add sensitive paths for which Zed will never predict edits."}).into_any_element()
                })
                .handler(move |window, cx| {
                    telemetry::event!(
                        "Edit Prediction Menu Action",
                        action = "configure_excluded_files",
                    );
                    if let Some(workspace) = Workspace::for_window(window, cx) {
                        let workspace = workspace.downgrade();
                        window
                            .spawn(cx, async |cx| {
                                open_disabled_globs_setting_in_editor(
                                    workspace,
                                    cx,
                                ).await
                            })
                            .detach_and_log_err(cx);
                    }
                }),
        ).item(
            ContextMenuEntry::new("View Docs")
                .icon(IconName::FileGeneric)
                .icon_color(Color::Muted)
                .handler(move |_, cx| {
                    telemetry::event!(
                        "Edit Prediction Menu Action",
                        action = "view_docs",
                    );
                    cx.open_url(PRIVACY_DOCS);
                })
        );

        if !self.editor_enabled.unwrap_or(true) {
            let icons = self.edit_prediction_provider.as_ref().map(|p| p.icons(cx));
            menu = menu.item(
                ContextMenuEntry::new("This file is excluded.")
                    .disabled(true)
                    .when_some(icons.map(|icons| icons.disabled), |item, disabled| {
                        item.icon(disabled)
                    })
                    .icon_size(IconSize::Small),
            );
        }

        if let Some(editor_focus_handle) = self.editor_focus_handle.clone() {
            menu = menu
                .separator()
                .header("Actions")
                .entry(
                    "Predict Edit at Cursor",
                    Some(Box::new(ShowEditPrediction)),
                    {
                        let editor_focus_handle = editor_focus_handle.clone();
                        move |window, cx| {
                            telemetry::event!(
                                "Edit Prediction Menu Action",
                                action = "predict_at_cursor",
                            );
                            editor_focus_handle.dispatch_action(&ShowEditPrediction, window, cx);
                        }
                    },
                )
                .context(editor_focus_handle);
        }

        menu
    }

    fn build_edit_prediction_context_menu(
        &self,
        provider: EditPredictionProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ContextMenu> {
        ContextMenu::build(window, cx, |mut menu, window, cx| {
            menu = self.build_language_settings_menu(menu, window, cx);
            menu = self.add_provider_switching_section(menu, provider, cx);


            let menu = self.add_configure_providers_item(menu);
            menu
        })
    }

    pub fn update_enabled(&mut self, editor: Entity<Editor>, cx: &mut Context<Self>) {
        let editor = editor.read(cx);
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        let suggestion_anchor = editor.selections.newest_anchor().start;
        let language = snapshot.language_at(suggestion_anchor);
        let file = snapshot.file_at(suggestion_anchor).cloned();
        self.editor_enabled = {
            let file = file.as_ref();
            Some(
                file.map(|file| {
                    all_language_settings(Some(file), cx)
                        .edit_predictions_enabled_for_file(file, cx)
                })
                .unwrap_or(true),
            )
        };
        self.editor_show_predictions = editor.edit_predictions_enabled();
        self.edit_prediction_provider = editor.edit_prediction_provider();
        self.language = language.cloned();
        self.file = file;
        self.editor_focus_handle = Some(editor.focus_handle(cx));

        cx.notify();
    }
}

impl StatusItemView for EditPredictionButton {
    fn set_active_pane_item(
        &mut self,
        item: Option<&dyn ItemHandle>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = item.and_then(|item| item.act_as::<Editor>(cx)) {
            self.editor_subscription = Some((
                cx.observe(&editor, Self::update_enabled),
                editor.entity_id().as_u64() as usize,
            ));
            self.update_enabled(editor, cx);
        } else {
            self.language = None;
            self.editor_subscription = None;
            self.editor_enabled = None;
        }
        cx.notify();
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        // This button is already gated on having a non-disabled edit
        // prediction provider, which the user manages through provider/AI
        // settings.
        None
    }
}

async fn open_disabled_globs_setting_in_editor(
    workspace: WeakEntity<Workspace>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let settings_editor = workspace
        .update_in(cx, |_, window, cx| {
            create_and_open_local_file(paths::settings_file(), window, cx, || {
                settings::initial_user_settings_content().as_ref().into()
            })
        })?
        .await?
        .downcast::<Editor>()
        .unwrap();

    settings_editor
        .downgrade()
        .update_in(cx, |item, window, cx| {
            let text = item.buffer().read(cx).snapshot(cx).text();

            let settings = cx.global::<SettingsStore>();

            // Ensure that we always have "edit_predictions { "disabled_globs": [] }"
            let Some(edits) = settings
                .edits_for_update(&text, |file| {
                    file.project
                        .all_languages
                        .edit_predictions
                        .get_or_insert_with(Default::default)
                        .disabled_globs
                        .get_or_insert_with(Vec::new);
                })
                .log_err()
            else {
                return;
            };

            if !edits.is_empty() {
                item.edit(
                    edits
                        .into_iter()
                        .map(|(r, s)| (MultiBufferOffset(r.start)..MultiBufferOffset(r.end), s)),
                    cx,
                );
            }

            let text = item.buffer().read(cx).snapshot(cx).text();

            static DISABLED_GLOBS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#""disabled_globs":\s*\[\s*(?P<content>(?:.|\n)*?)\s*\]"#).unwrap()
            });
            // Only capture [...]
            let range = DISABLED_GLOBS_REGEX.captures(&text).and_then(|captures| {
                captures
                    .name("content")
                    .map(|inner_match| inner_match.start()..inner_match.end())
            });
            if let Some(range) = range {
                let range = MultiBufferOffset(range.start)..MultiBufferOffset(range.end);
                item.change_selections(
                    SelectionEffects::scroll(Autoscroll::newest()),
                    window,
                    cx,
                    |selections| {
                        selections.select_ranges(vec![range]);
                    },
                );
            }
        })?;

    anyhow::Ok(())
}

pub fn set_completion_provider(fs: Arc<dyn Fs>, cx: &mut App, provider: EditPredictionProvider) {
    update_settings_file(fs, cx, move |settings, _| {
        settings
            .project
            .all_languages
            .edit_predictions
            .get_or_insert_default()
            .provider = Some(provider);
    });
}

pub fn get_available_providers(cx: &mut App) -> Vec<EditPredictionProvider> {
    let mut providers = Vec::new();


    if all_language_settings(None, cx)
        .edit_predictions
        .ollama
        .is_some()
    {
        providers.push(EditPredictionProvider::Ollama);
    }

    if all_language_settings(None, cx)
        .edit_predictions
        .open_ai_compatible_api
        .is_some()
    {
        providers.push(EditPredictionProvider::OpenAiCompatibleApi);
    }

    providers
}

fn toggle_show_edit_predictions_for_language(
    language: Arc<Language>,
    fs: Arc<dyn Fs>,
    cx: &mut App,
) {
    let show_edit_predictions =
        all_language_settings(None, cx).show_edit_predictions(Some(&language), cx);
    update_settings_file(fs, cx, move |settings, _| {
        settings
            .project
            .all_languages
            .languages
            .0
            .entry(language.name().0.to_string())
            .or_default()
            .show_edit_predictions = Some(!show_edit_predictions);
    });
}

fn toggle_edit_prediction_mode(fs: Arc<dyn Fs>, mode: EditPredictionsMode, cx: &mut App) {
    let settings = AllLanguageSettings::get_global(cx);
    let current_mode = settings.edit_predictions_mode();

    if current_mode != mode {
        update_settings_file(fs, cx, move |settings, _cx| {
            if let Some(edit_predictions) = settings.project.all_languages.edit_predictions.as_mut()
            {
                edit_predictions.mode = Some(mode);
            } else {
                settings.project.all_languages.edit_predictions =
                    Some(settings::EditPredictionSettingsContent {
                        mode: Some(mode),
                        ..Default::default()
                    });
            }
        });
    }
}

