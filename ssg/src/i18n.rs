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
    /// 渲染期间遇到的缺失 key（"lang/key"）。构建收尾检查非空即失败——
    /// 缺文案是模板笔误，不该以「构建成功 + 线上空白文案」的形式溜到线上。
    /// Mutex 是因为 translate 经 Arc<I18n> 在模板函数里被调用，只有 &self。
    missing: std::sync::Mutex<std::collections::BTreeSet<String>>,
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
        Ok(I18n {
            tables,
            missing: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        })
    }

    /// 找不到 key 时返回空串占位，同时记入 missing 清单（构建收尾统一报错）
    pub fn translate(&self, lang: &str, key: &str, args: &HashMap<String, String>) -> String {
        let Some(tr) = self.tables.get(lang).and_then(|t| t.get(key)) else {
            eprintln!("警告: i18n 缺少 {lang}/{key}");
            self.missing
                .lock()
                .expect("missing-key 集合锁中毒")
                .insert(format!("{lang}/{key}"));
            return String::new();
        };
        let template = match (&tr.one, args.get("Count")) {
            (Some(one), Some(count)) if count == "1" => one,
            _ => &tr.other,
        };
        interpolate(template, args)
    }

    /// 渲染全程累计的缺失 key（已排序去重），空表示文案完整。
    pub fn missing_keys(&self) -> Vec<String> {
        self.missing
            .lock()
            .expect("missing-key 集合锁中毒")
            .iter()
            .cloned()
            .collect()
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
