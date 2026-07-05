//! go-i18n TOML 文案的最小实现：`other`/`one` 两种形态 + `{{ .Key }}` 插值。

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub struct Translation {
    pub other: String,
    pub one: Option<String>,
}

pub struct I18n {
    // lang -> key -> 文案
    tables: HashMap<String, HashMap<String, Translation>>,
}

impl I18n {
    pub fn load(source: &Path, langs: &[String]) -> Result<Self> {
        let mut tables = HashMap::new();
        for lang in langs {
            let path = source.join("i18n").join(format!("{lang}.toml"));
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 {} 失败", path.display()))?;
            let doc: toml::Value = raw.parse().context("解析 i18n TOML 失败")?;
            let mut table = HashMap::new();
            if let Some(entries) = doc.as_table() {
                for (key, val) in entries {
                    table.insert(
                        key.clone(),
                        Translation {
                            other: val
                                .get("other")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            one: val.get("one").and_then(|v| v.as_str()).map(String::from),
                        },
                    );
                }
            }
            tables.insert(lang.clone(), table);
        }
        Ok(I18n { tables })
    }

    /// Hugo 的 i18n：找不到 key 时返回空串（并在构建日志里能看出缺失）
    pub fn translate(&self, lang: &str, key: &str, args: &HashMap<String, String>) -> String {
        let Some(tr) = self.tables.get(lang).and_then(|t| t.get(key)) else {
            eprintln!("警告: i18n 缺少 {lang}/{key}");
            return String::new();
        };
        let template = match (&tr.one, args.get("Count")) {
            (Some(one), Some(count)) if count == "1" => one,
            _ => &tr.other,
        };
        interpolate(template, args)
    }
}

/// 替换文案中的 `{{ .Name }}` 占位（容忍空白差异，如 `{{.Count}}`）
fn interpolate(template: &str, args: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = after[..end].trim();
        let name = inner.trim_start_matches('.');
        match args.get(name) {
            Some(value) => out.push_str(value),
            None => {
                // 保留原样，便于发现漏传的参数
                out.push_str(&rest[start..start + 2 + end + 2]);
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}
