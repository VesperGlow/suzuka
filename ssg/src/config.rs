use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub struct Language {
    pub code: String,
    pub locale: String,
    pub description: String,
    pub weight: i64,
    pub has_cjk: bool,
}

impl Language {
    /// 默认语言位于根路径，其余语言带 /<code>/ 前缀。
    pub fn url_prefix(&self, default_lang: &str) -> String {
        if self.code == default_lang {
            String::new()
        } else {
            format!("/{}", self.code)
        }
    }

    pub fn home_rel(&self, default_lang: &str) -> String {
        format!("{}/", self.url_prefix(default_lang))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    title: String,
    base_url: String,
    default_language: String,
    summary_length: usize,
    languages: BTreeMap<String, LanguageFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageFile {
    locale: String,
    description: String,
    weight: i64,
    #[serde(default)]
    has_cjk: bool,
}

pub struct SiteConfig {
    pub title: String,
    pub base_url: String,
    pub default_lang: String,
    pub summary_length: usize,
    pub languages: Vec<Language>,
}

impl SiteConfig {
    pub fn load(source: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(source.join("site.toml")).context("读取 site.toml 失败")?;
        Self::parse(&raw)
    }

    fn parse(raw: &str) -> Result<Self> {
        let file: ConfigFile = toml::from_str(raw).context("解析 site.toml 失败")?;
        if file.title.trim().is_empty() {
            bail!("站点 title 不能为空");
        }
        let base_url = file.base_url.trim_end_matches('/').to_string();
        let origin = base_url
            .strip_prefix("https://")
            .or_else(|| base_url.strip_prefix("http://"))
            .context("base_url 必须是 http(s) 站点根地址")?;
        if origin.is_empty() || origin.contains(['/', '?', '#', ' ', '"', '<', '>']) {
            bail!("base_url 必须是 http(s) 站点根地址");
        }
        if !file.languages.contains_key(&file.default_language) {
            bail!("default_language 必须对应已配置的语言");
        }
        let mut languages: Vec<_> = file
            .languages
            .into_iter()
            .map(|(code, lang)| Language {
                code,
                locale: lang.locale,
                description: lang.description,
                weight: lang.weight,
                has_cjk: lang.has_cjk,
            })
            .collect();
        languages.sort_by_key(|lang| lang.weight);
        Ok(Self {
            title: file.title,
            base_url,
            default_lang: file.default_language,
            summary_length: file.summary_length,
            languages,
        })
    }

    pub fn language(&self, code: &str) -> &Language {
        self.languages
            .iter()
            .find(|lang| lang.code == code)
            .expect("未知语言代码")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_ignored_fields_and_invalid_deployment_urls() {
        let raw = include_str!("../../site.toml");
        let config = SiteConfig::parse(raw).unwrap();
        assert!(!config.base_url.ends_with('/'));
        assert!(SiteConfig::parse(&format!("legacy_theme = true\n{raw}")).is_err());
        assert!(SiteConfig::parse(&raw.replace(
            &format!("{}/", config.base_url),
            &format!("{}/blog/", config.base_url),
        ))
        .is_err());
        assert!(SiteConfig::parse(&raw.replace(
            &format!("default_language = '{}'", config.default_lang),
            "default_language = 'missing'",
        ))
        .is_err());
    }
}
