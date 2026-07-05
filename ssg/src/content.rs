//! 内容加载：扫描 content/，解析 front matter，配对翻译，计算 URL。

use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Hugo RelPermalink 对路径段的转义集合：非 ASCII 与空格等百分号编码
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'%');

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct FrontMatter {
    pub title: String,
    pub slug: Option<String>,
    pub date: Option<serde_yaml::Value>,
    pub draft: bool,
    pub aliases: Vec<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub spoiler: Option<String>,
    pub url: Option<String>,
    pub layout: Option<String>,
    #[serde(rename = "redirectTo")]
    pub redirect_to: Option<String>,
}

pub struct Resource {
    /// 文件名，如 "001.webp"
    pub name: String,
    pub disk_path: PathBuf,
    pub width: u32,
    pub height: u32,
}

pub struct RawPage {
    pub lang: String,
    /// "posts" | "pages" | "section"（_index）
    pub kind: PageKind,
    /// posts 的 bundle 目录名或 pages 的文件主干名，用于翻译配对
    pub key: String,
    pub fm: FrontMatter,
    pub body: String,
    pub date: DateTime<FixedOffset>,
}

#[derive(PartialEq, Clone, Copy)]
pub enum PageKind {
    Post,
    Page,
    Section,
}

pub struct PostBundle {
    pub key: String,
    /// URL 路径段（slug 优先，否则目录名）
    pub slug: String,
    pub resources: Vec<Resource>,
    /// lang -> 页面
    pub versions: Vec<RawPage>,
}

pub struct Content {
    pub posts: Vec<PostBundle>,
    pub pages: Vec<RawPage>,
}

pub fn load(source: &Path, default_lang: &str) -> Result<Content> {
    let mut posts = Vec::new();
    let posts_dir = source.join("content").join("posts");
    for entry in std::fs::read_dir(&posts_dir).context("读取 content/posts 失败")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let key = entry.file_name().to_string_lossy().to_string();

        let mut versions = Vec::new();
        let mut resources = Vec::new();
        for file in std::fs::read_dir(&dir)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().to_string();
            let path = file.path();
            if name == "index.md" || (name.starts_with("index.") && name.ends_with(".md")) {
                let lang = if name == "index.md" {
                    default_lang.to_string()
                } else {
                    name.trim_start_matches("index.")
                        .trim_end_matches(".md")
                        .to_string()
                };
                if let Some(page) = parse_page(&path, &lang, PageKind::Post, &key)? {
                    versions.push(page);
                }
            } else if !name.ends_with(".md") {
                let (width, height) = match imagesize::size(&path) {
                    Ok(dim) => (dim.width as u32, dim.height as u32),
                    Err(_) => (0, 0),
                };
                resources.push(Resource {
                    name,
                    disk_path: path,
                    width,
                    height,
                });
            }
        }
        if versions.is_empty() {
            continue;
        }
        resources.sort_by(|a, b| a.name.cmp(&b.name));
        let slug = versions
            .iter()
            .find(|v| v.lang == default_lang)
            .or(versions.first())
            .and_then(|v| v.fm.slug.clone())
            .unwrap_or_else(|| key.clone());
        posts.push(PostBundle {
            key,
            slug,
            resources,
            versions,
        });
    }
    // 站内的默认排序：日期倒序（新→旧）
    posts.sort_by(|a, b| bundle_date(b).cmp(&bundle_date(a)));

    let mut pages = Vec::new();
    let pages_dir = source.join("content").join("pages");
    for entry in std::fs::read_dir(&pages_dir).context("读取 content/pages 失败")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        let stem = name.trim_end_matches(".md");
        let (key, lang, kind) = match stem.rsplit_once('.') {
            Some((base, lang_part)) if lang_part.len() <= 5 && base != "_index" => {
                (base.to_string(), lang_part.to_string(), PageKind::Page)
            }
            Some(("_index", lang_part)) => {
                ("_index".to_string(), lang_part.to_string(), PageKind::Section)
            }
            _ if stem == "_index" => ("_index".to_string(), default_lang.to_string(), PageKind::Section),
            _ => (stem.to_string(), default_lang.to_string(), PageKind::Page),
        };
        if let Some(page) = parse_page(&entry.path(), &lang, kind, &key)? {
            pages.push(page);
        }
    }

    Ok(Content { posts, pages })
}

fn bundle_date(b: &PostBundle) -> DateTime<FixedOffset> {
    b.versions
        .iter()
        .map(|v| v.date)
        .max()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap().fixed_offset())
}

fn parse_page(path: &Path, lang: &str, kind: PageKind, key: &str) -> Result<Option<RawPage>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path.display()))?;
    let (fm_raw, body) = split_front_matter(&raw)
        .with_context(|| format!("{} 缺少 front matter", path.display()))?;
    let fm: FrontMatter = serde_yaml::from_str(fm_raw)
        .with_context(|| format!("解析 {} front matter 失败", path.display()))?;
    if fm.draft {
        return Ok(None);
    }
    let date = parse_date(fm.date.as_ref())
        .with_context(|| format!("解析 {} 日期失败", path.display()))?;
    Ok(Some(RawPage {
        lang: lang.to_string(),
        kind,
        key: key.to_string(),
        fm,
        body: body.to_string(),
        date,
    }))
}

fn split_front_matter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---")?;
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let body = &rest[end + 4..];
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    Some((fm, body))
}

fn parse_date(value: Option<&serde_yaml::Value>) -> Result<DateTime<FixedOffset>> {
    let epoch = || Utc.timestamp_opt(0, 0).unwrap().fixed_offset();
    let Some(value) = value else {
        return Ok(epoch());
    };
    let text = match value {
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other)?.trim().to_string(),
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
        return Ok(dt);
    }
    if let Ok(d) = NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
        // Hugo 对无时区的纯日期按 UTC 零点处理
        return Ok(d
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .fixed_offset());
    }
    bail!("无法解析日期: {text}")
}

/// Hugo urlize：小写、空格转连字符（term 页路径用）
pub fn urlize(s: &str) -> String {
    s.to_lowercase().replace(' ', "-")
}

/// RelPermalink 形态：对路径段做百分号编码（保留 '/'）
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| utf8_percent_encode(seg, PATH_SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Go url.QueryEscape：空格转 '+'，其余非安全字符 %XX（大写十六进制）
pub fn query_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Hugo 的中日韩词数统计：CJK 字符逐字计数，其余按空白分词
pub fn word_count_cjk(plain: &str) -> usize {
    let mut count = 0;
    for word in plain.split_whitespace() {
        let mut latin_run = false;
        for ch in word.chars() {
            if is_cjk(ch) {
                if latin_run {
                    count += 1;
                    latin_run = false;
                }
                count += 1;
            } else {
                latin_run = true;
            }
        }
        if latin_run {
            count += 1;
        }
    }
    count
}

pub fn word_count_latin(plain: &str) -> usize {
    plain.split_whitespace().count()
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x2E80..=0x9FFF | 0xA000..=0xA4CF | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}
