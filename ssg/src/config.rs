use anyhow::{Context, Result};
use std::path::Path;

pub struct MenuItem {
    pub identifier: String,
    pub name: String,
    pub page_ref: String,
    pub weight: i64,
}

pub struct Language {
    /// hugo.toml 里的语言键，如 "zh-cn"、"en"
    pub code: String,
    pub locale: String,
    pub title: String,
    pub description: String,
    pub weight: i64,
    pub has_cjk: bool,
    pub menus: Vec<MenuItem>,
}

impl Language {
    /// 默认语言在根路径，其余语言带 /<lang>/ 前缀（defaultContentLanguageInSubdir=false）
    pub fn url_prefix(&self, default_lang: &str) -> String {
        if self.code == default_lang {
            String::new()
        } else {
            format!("/{}", self.code)
        }
    }
}

pub struct SiteConfig {
    pub base_url: String,
    pub default_lang: String,
    pub author: String,
    pub images: Vec<String>,
    pub summary_length: usize,
    pub languages: Vec<Language>,
}

impl SiteConfig {
    pub fn load(source: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(source.join("hugo.toml"))
            .context("读取 hugo.toml 失败")?;
        let doc: toml::Value = raw.parse().context("解析 hugo.toml 失败")?;

        let base_url = doc["baseURL"]
            .as_str()
            .context("hugo.toml 缺少 baseURL")?
            .trim_end_matches('/')
            .to_string();
        let default_lang = doc["defaultContentLanguage"]
            .as_str()
            .unwrap_or("en")
            .to_string();
        let params = doc.get("params");
        let author = params
            .and_then(|p| p.get("author"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let images = params
            .and_then(|p| p.get("images"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let summary_length = doc
            .get("summaryLength")
            .and_then(|v| v.as_integer())
            .unwrap_or(70) as usize;

        let mut languages = Vec::new();
        if let Some(langs) = doc.get("languages").and_then(|v| v.as_table()) {
            for (code, lang) in langs {
                let lang_params = lang.get("params");
                let mut menus = Vec::new();
                if let Some(items) = lang
                    .get("menus")
                    .and_then(|m| m.get("main"))
                    .and_then(|v| v.as_array())
                {
                    for item in items {
                        menus.push(MenuItem {
                            identifier: str_field(item, "identifier"),
                            name: str_field(item, "name"),
                            page_ref: str_field(item, "pageRef"),
                            weight: item
                                .get("weight")
                                .and_then(|v| v.as_integer())
                                .unwrap_or(0),
                        });
                    }
                    menus.sort_by_key(|m| m.weight);
                }
                languages.push(Language {
                    code: code.clone(),
                    locale: str_field(lang, "locale"),
                    title: str_field(lang, "title"),
                    description: lang_params
                        .map(|p| str_field(p, "description"))
                        .unwrap_or_default(),
                    weight: lang.get("weight").and_then(|v| v.as_integer()).unwrap_or(0),
                    has_cjk: lang
                        .get("hasCJKLanguage")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    menus,
                });
            }
        }
        languages.sort_by_key(|l| l.weight);

        Ok(SiteConfig {
            base_url,
            default_lang,
            author,
            images,
            summary_length,
            languages,
        })
    }

    pub fn language(&self, code: &str) -> &Language {
        self.languages
            .iter()
            .find(|l| l.code == code)
            .expect("未知语言代码")
    }
}

fn str_field(v: &toml::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}
