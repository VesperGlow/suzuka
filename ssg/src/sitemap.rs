//! sitemap.xml：根 sitemapindex 汇总各语言，各语言一份 urlset。

use crate::build::LangData;
use crate::config::SiteConfig;
use crate::content::{Content, PageKind};
use crate::gotime;
use crate::meta::alias_html;
use crate::output::write_file;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// sitemap.xml：根 sitemapindex 汇总各语言，各语言一份 urlset
pub(crate) fn write_sitemaps(
    config: &SiteConfig,
    content: &Content,
    lang_data: &HashMap<String, LangData>,
    dest: &Path,
) -> Result<()> {
    let mut entries_by_lang: HashMap<String, Vec<SitemapEntry>> = HashMap::new();
    for lang in &config.languages {
        let prefix = lang.url_prefix(&config.default_lang);
        let data = &lang_data[&lang.code];
        let strip = |rel: &str| -> String {
            if !prefix.is_empty() {
                rel.strip_prefix(&prefix).unwrap_or(rel).to_string()
            } else {
                rel.to_string()
            }
        };
        let mut entries = Vec::new();

        // 首页
        let home_rel = lang.home_rel(&config.default_lang);
        entries.push(SitemapEntry {
            match_key: strip(&home_rel),
            rel: home_rel,
            lastmod: data
                .newest_date
                .map(|d| gotime::format(&d, gotime::RFC3339)),
        });

        // 文章（frontmatter 声明 sitemap.disable 的除外）
        for card in data.cards.iter().filter(|c| !c.sitemap_disable) {
            entries.push(SitemapEntry {
                match_key: strip(&card.rel),
                rel: card.rel.clone(),
                lastmod: Some(gotime::format(&card.date, gotime::RFC3339)),
            });
        }

        // about / guestbook
        for raw_page in content
            .pages
            .iter()
            .filter(|p| p.lang == lang.code && p.kind == PageKind::Page && !p.fm.sitemap.disable)
        {
            let Some(url) = raw_page.fm.url.clone() else {
                continue;
            };
            let lastmod = if raw_page.date.timestamp() == 0 {
                None
            } else {
                Some(gotime::format(&raw_page.date, gotime::RFC3339))
            };
            entries.push(SitemapEntry {
                match_key: strip(&url),
                rel: url,
                lastmod,
            });
        }

        // archives
        if content
            .archives
            .iter()
            .any(|p| p.lang == lang.code && !p.fm.sitemap.disable)
        {
            let rel = format!("{prefix}/archives/");
            entries.push(SitemapEntry {
                match_key: strip(&rel),
                rel,
                lastmod: None,
            });
        }

        // 分类/标签：列表页 + term 页
        for (taxonomy, terms) in [("categories", &data.categories), ("tags", &data.tags)] {
            let list_rel = format!("{prefix}/{taxonomy}/");
            entries.push(SitemapEntry {
                match_key: strip(&list_rel),
                rel: list_rel,
                lastmod: data
                    .newest_date
                    .map(|d| gotime::format(&d, gotime::RFC3339)),
            });
            for term in terms.iter() {
                let lastmod = term
                    .posts
                    .iter()
                    .map(|&i| data.cards[i].date)
                    .max()
                    .map(|d| gotime::format(&d, gotime::RFC3339));
                entries.push(SitemapEntry {
                    match_key: strip(&term.rel),
                    rel: term.rel.clone(),
                    lastmod,
                });
            }
        }

        entries_by_lang.insert(lang.code.clone(), entries);
    }

    // alts_by_key：match_key -> 按 config.languages 固定顺序排列的
    // (hreflang, rel) 列表，只有 len>1 才真的输出 xhtml:link
    let mut alts_by_key: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for lang in &config.languages {
        let hreflang = if lang.code == config.default_lang {
            lang.locale.clone()
        } else {
            lang.code.clone()
        };
        for e in &entries_by_lang[&lang.code] {
            alts_by_key
                .entry(e.match_key.clone())
                .or_default()
                .push((hreflang.clone(), e.rel.clone()));
        }
    }

    // 每种语言的 sitemap.xml 固定挂在 /<语言 code>/sitemap.xml 下——默认
    // 语言实际 URL 没有前缀，所以在语言 code 路径下额外放一个跳回真实
    // 首页的重定向桩
    let mut index_langs = Vec::new();
    for lang in &config.languages {
        let entries = &entries_by_lang[&lang.code];
        let urlset = render_sitemap_urlset(&config.base_url, entries, &alts_by_key);
        write_file(dest, &format!("{}/sitemap.xml", lang.code), &urlset)?;
        if lang.code == config.default_lang {
            let home_permalink = format!("{}/", config.base_url);
            write_file(
                dest,
                &format!("{}/index.html", lang.code),
                &alias_html(&lang.locale, &home_permalink),
            )?;
        }
        let lang_newest = entries.iter().filter_map(|e| e.lastmod.clone()).max();
        index_langs.push((format!("/{}", lang.code), lang_newest));
    }
    let index_langs_ref: Vec<(&str, Option<&str>)> = index_langs
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_deref()))
        .collect();
    write_file(
        dest,
        "sitemap.xml",
        &render_sitemap_index(&config.base_url, &index_langs_ref),
    )
}

/// 一条 sitemap.xml `<url>` 记录
struct SitemapEntry {
    /// 带语言前缀的根相对路径，如 "/en/tags/galgame/"
    rel: String,
    /// lastmod：None 表示页面没有真实日期，不输出这一行
    lastmod: Option<String>,
    /// 去掉语言前缀后的路径，用来跟其他语言的条目配对判断是否互为翻译
    match_key: String,
}

fn render_sitemap_index(base_url: &str, langs: &[(&str, Option<&str>)]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for (prefix, lastmod) in langs {
        s.push_str("  <sitemap>\n");
        s.push_str(&format!("    <loc>{base_url}{prefix}/sitemap.xml</loc>\n"));
        if let Some(lm) = lastmod {
            s.push_str(&format!("    <lastmod>{lm}</lastmod>\n"));
        }
        s.push_str("  </sitemap>\n");
    }
    s.push_str("</sitemapindex>\n");
    s
}

/// alts_by_key：match_key -> 这个路径在各语言下的 (hreflang, rel)，按
/// config.languages 的固定顺序（zh 在前）——不管在渲染哪个语言的 urlset，
/// alternate 链接的先后顺序都固定
fn render_sitemap_urlset(
    base_url: &str,
    entries: &[SitemapEntry],
    alts_by_key: &HashMap<String, Vec<(String, String)>>,
) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n");
    let mut sorted: Vec<&SitemapEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| b.lastmod.cmp(&a.lastmod));
    for e in sorted {
        s.push_str("  <url>\n");
        s.push_str(&format!("    <loc>{base_url}{}</loc>\n", e.rel));
        if let Some(lm) = &e.lastmod {
            s.push_str(&format!("    <lastmod>{lm}</lastmod>\n"));
        }
        if let Some(alts) = alts_by_key.get(&e.match_key) {
            if alts.len() > 1 {
                for (hreflang, rel) in alts {
                    s.push_str(&format!(
                        "    <xhtml:link rel=\"alternate\" hreflang=\"{hreflang}\" href=\"{base_url}{rel}\"/>\n"
                    ));
                }
            }
        }
        s.push_str("  </url>\n");
    }
    s.push_str("</urlset>\n");
    s
}
