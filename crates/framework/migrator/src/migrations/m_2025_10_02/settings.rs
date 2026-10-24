use anyhow::Result;
use serde_json::Value;

use crate::migrations::migrate_language_setting;

pub fn remove_formatters_on_save(value: &mut Value) -> Result<()> {
    migrate_language_setting(value, remove_formatters_on_save_inner)
}

/// Prettier formatting has been removed; settings that still name `"prettier"`
/// as a formatter would migrate to a value that no longer exists. Rewrite them
/// to `"auto"`, which is what Prettier fell back to when it did not apply.
fn replace_prettier_formatter(value: &mut Value) {
    match value {
        Value::String(formatter) if formatter == "prettier" => *formatter = "auto".to_string(),
        Value::Array(formatters) => {
            for formatter in formatters {
                if let Value::String(name) = formatter {
                    if name == "prettier" {
                        *name = "auto".to_string();
                    }
                }
            }
        }
        _ => {}
    }
}

fn remove_formatters_on_save_inner(value: &mut Value, path: &[&str]) -> Result<()> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(mut format_on_save) = obj.get("format_on_save").cloned() else {
        return Ok(());
    };
    let is_format_on_save_set_to_formatter = format_on_save.as_str().map_or(true, |s| {
        s != "on" && s != "off" && s != "modifications" && s != "modifications_if_available"
    });
    if !is_format_on_save_set_to_formatter {
        return Ok(());
    }

    replace_prettier_formatter(&mut format_on_save);

    fn fmt_path(path: &[&str], key: &str) -> String {
        let mut path = path.to_vec();
        path.push(key);
        path.join(".")
    }

    anyhow::ensure!(
        obj.get("formatter").is_none(),
        r#"Setting formatters in both "format_on_save" and "formatter" is deprecated. Please migrate the formatters from {} into {}"#,
        fmt_path(path, "format_on_save"),
        fmt_path(path, "formatter")
    );

    obj.insert("format_on_save".to_string(), serde_json::json!("on"));
    obj.insert("formatter".to_string(), format_on_save);

    Ok(())
}
