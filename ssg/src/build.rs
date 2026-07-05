//! 构建编排：内容 → 页面模型 → minijinja 渲染 → 写盘。

use crate::config::SiteConfig;
use crate::content::{self, Content, PageKind, PostBundle, RawPage};
use crate::gotime;
use crate::i18n::I18n;
use crate::markdown;
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, FixedOffset};
use minijinja::value::{Kwargs, Value};
use minijinja::{path_loader, Environment, State, UndefinedBehavior};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Serialize, Clone)]
struct TermRef {
    title: String,
    rel: String,
}

#[derive(Serialize, Clone)]
struct PostRef {
    title: String,
    rel: String,
    date_iso: String,
    date_display: String,
    date_mmdd: String,
    year: String,
    category: Option<TermRef>,
}

#[derive(Serialize, Clone)]
struct TranslationRef {
    lang: String,
    locale: String,
    permalink: String,
    rel: String,
}

#[derive(Serialize, Clone)]
struct YearGroup {
    year: String,
    posts: Vec<PostRef>,
}

#[derive(Serialize)]
struct SiteCtx {
    title: String,
    lang: String,
    locale: String,
    description: String,
    base_url: String,
    home_rel: String,
    author: String,
    current_year: i32,
    rss_rel: String,
    jsonfeed_rel: String,
    about_rel: String,
    guestbook_rel: String,
}

#[derive(Serialize, Default, Clone)]
struct PageCtx {
    kind: String,
    is_home: bool,
    is_post: bool,
    layout: String,
    section: String,
    title: String,
    description_meta: String,
    description: String,
    rel_permalink: String,
    permalink: String,
    date_iso: String,
    date_display: String,
    reading_time: usize,
    tags: Vec<TermRef>,
    categories: Vec<TermRef>,
    spoiler: Option<String>,
    content_html: String,
    toc_html: String,
    has_toc: bool,
    prev: Option<PostRef>,
    next: Option<PostRef>,
    all_translations: Vec<TranslationRef>,
    /// og/twitter/schema/JSON-LD 的整块 head 输出（Rust 侧逐字节构造）
    internal_meta: String,
}

struct LangData {
    posts: Vec<PageCtx>,
    post_refs: Vec<PostRef>,
    timeline: Vec<YearGroup>,
    newest_date: Option<DateTime<FixedOffset>>,
}

pub fn build(source: &Path, dest: &Path) -> Result<()> {
    let config = SiteConfig::load(source)?;
    let lang_codes: Vec<String> = config.languages.iter().map(|l| l.code.clone()).collect();
    let i18n = Arc::new(I18n::load(source, &lang_codes)?);
    let content = content::load(source, &config.default_lang)?;
    let assets = crate::assets::build(source, dest)?;

    // 文章 bundle 的图片资源只按默认语言路径发布一份（与 Hugo 行为一致）
    for bundle in &content.posts {
        let out_dir = dest.join("posts").join(&bundle.slug);
        std::fs::create_dir_all(&out_dir)?;
        for res in &bundle.resources {
            std::fs::copy(&res.disk_path, out_dir.join(&res.name))?;
        }
    }

    let mut env = Environment::new();
    env.set_loader(path_loader(source.join("ssg").join("templates")));
    env.set_keep_trailing_newline(true);
    env.set_undefined_behavior(UndefinedBehavior::Chainable);
    {
        let i18n = i18n.clone();
        env.add_function(
            "t",
            move |state: &State, key: String, kwargs: Kwargs| -> String {
                let lang = state
                    .lookup("lang")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut args = HashMap::new();
                for name in kwargs.args() {
                    if let Ok(v) = kwargs.get::<Value>(name) {
                        args.insert(name.to_string(), v.to_string());
                    }
                }
                i18n.translate(&lang, &key, &args)
            },
        );
    }
    {
        let urls = assets.urls.clone();
        env.add_function("asset", move |path: String| -> String {
            urls.get(&path).cloned().unwrap_or_else(|| {
                eprintln!("警告: 找不到资源 {path}");
                format!("/{path}")
            })
        });
    }
    env.add_filter("urlquery", |s: String| content::query_escape(&s));

    // 每种语言的独立页 URL（sidebar / guestbook 链接用）
    let special_rel = |key: &str, lang: &str| -> String {
        let prefix = config.language(lang).url_prefix(&config.default_lang);
        content
            .pages
            .iter()
            .find(|p| p.key == key && p.lang == lang)
            .and_then(|p| p.fm.url.clone())
            .map(|u| format!("{prefix}{u}"))
            .unwrap_or_default()
    };

    let mut lang_data: HashMap<String, LangData> = HashMap::new();
    for lang in &config.languages {
        lang_data.insert(
            lang.code.clone(),
            build_lang_posts(&config, &content, &i18n, &lang.code)?,
        );
    }

    for lang in &config.languages {
        let prefix = lang.url_prefix(&config.default_lang);
        let data = &lang_data[&lang.code];
        let site = SiteCtx {
            title: lang.title.clone(),
            lang: lang.code.clone(),
            locale: lang.locale.clone(),
            description: lang.description.clone(),
            base_url: config.base_url.clone(),
            home_rel: if prefix.is_empty() {
                "/".into()
            } else {
                format!("{prefix}/")
            },
            author: config.author.clone(),
            current_year: chrono::Utc::now().year(),
            rss_rel: format!("{prefix}/feed.xml"),
            jsonfeed_rel: format!("{prefix}/feed.json"),
            about_rel: special_rel("about", &lang.code),
            guestbook_rel: special_rel("guestbook", &lang.code),
        };

        // 首页
        let home = home_ctx(&config, lang, data, &content)?;
        render_to(
            &env,
            "home.html",
            dest,
            &format!("{}index.html", site.home_rel.trim_start_matches('/')),
            minijinja::context! {
                lang => lang.code.clone(),
                site => &site,
                page => &home,
                posts => &data.post_refs,
                timeline => &data.timeline,
                theme_js => &assets.theme_js,
            },
        )?;

        // 文章页 + 别名跳转页
        for (idx, page) in data.posts.iter().enumerate() {
            let out_rel = format!("{}index.html", page.rel_permalink.trim_start_matches('/'));
            render_to(
                &env,
                "single.html",
                dest,
                &out_rel,
                minijinja::context! {
                    lang => lang.code.clone(),
                    site => &site,
                    page => page,
                    posts => &data.post_refs,
                    timeline => &data.timeline,
                    theme_js => &assets.theme_js,
                },
            )?;
            let _ = idx;
        }

        for bundle in &content.posts {
            let Some(version) = bundle.versions.iter().find(|v| v.lang == lang.code) else {
                continue;
            };
            let permalink = format!(
                "{}{prefix}/posts/{}/",
                config.base_url,
                content::encode_path(&bundle.slug)
            );
            for alias in &version.fm.aliases {
                let alias_rel = format!("{prefix}{alias}");
                let out = format!("{}index.html", alias_rel.trim_start_matches('/'));
                write_file(dest, &out, &alias_html(&lang.locale, &permalink))?;
            }
        }
    }
    println!("构建完成 → {}", dest.display());
    Ok(())
}

fn build_lang_posts(
    config: &SiteConfig,
    content: &Content,
    i18n: &I18n,
    lang_code: &str,
) -> Result<LangData> {
    let lang = config.language(lang_code);
    let prefix = lang.url_prefix(&config.default_lang);
    let date_layout = i18n.translate(lang_code, "dateFormat", &HashMap::new());

    // 该语言下的文章（已按日期倒序）
    let mut items: Vec<(&PostBundle, &RawPage)> = Vec::new();
    for bundle in &content.posts {
        if let Some(v) = bundle.versions.iter().find(|v| v.lang == lang_code) {
            items.push((bundle, v));
        }
    }
    items.sort_by(|a, b| b.1.date.cmp(&a.1.date));

    let term_ref = |taxonomy: &str, name: &str| TermRef {
        title: name.to_string(),
        rel: format!(
            "{prefix}/{taxonomy}/{}/",
            content::encode_path(&content::urlize(name))
        ),
    };

    let mut post_refs = Vec::new();
    for (bundle, page) in &items {
        post_refs.push(PostRef {
            title: page.fm.title.clone(),
            rel: format!("{prefix}/posts/{}/", content::encode_path(&bundle.slug)),
            date_iso: gotime::format(&page.date, "2006-01-02"),
            date_display: gotime::format(&page.date, &date_layout),
            date_mmdd: gotime::format(&page.date, "01.02"),
            year: gotime::format(&page.date, "2006"),
            category: page
                .fm
                .categories
                .first()
                .map(|c| term_ref("categories", c)),
        });
    }

    // 侧栏时间线：按年分组（新→旧）
    let mut timeline: Vec<YearGroup> = Vec::new();
    for post in &post_refs {
        match timeline.last_mut() {
            Some(group) if group.year == post.year => group.posts.push(post.clone()),
            _ => timeline.push(YearGroup {
                year: post.year.clone(),
                posts: vec![post.clone()],
            }),
        }
    }

    let about_permalink = content
        .pages
        .iter()
        .find(|p| p.key == "about" && p.lang == lang_code)
        .and_then(|p| p.fm.url.clone())
        .map(|u| format!("{}{prefix}{u}", config.base_url));

    let mut posts = Vec::new();
    for (idx, (bundle, page)) in items.iter().enumerate() {
        let rel = format!("{prefix}/posts/{}/", content::encode_path(&bundle.slug));
        let bundle_rel = format!("/posts/{}/", content::encode_path(&bundle.slug));
        let rendered = markdown::render(&page.body, &bundle.resources, &bundle_rel);
        let word_count = if lang.has_cjk {
            content::word_count_cjk(&rendered.plain)
        } else {
            content::word_count_latin(&rendered.plain)
        };
        let reading_time = if lang.has_cjk {
            (word_count + 500) / 501
        } else {
            (word_count + 212) / 213
        };
        let h2_count = rendered.toc.iter().filter(|t| t.level == 2).count();
        let has_toc = h2_count > 1;
        let permalink = format!("{}{rel}", config.base_url);
        let description = page.fm.description.clone().unwrap_or_default();
        let date_rfc = gotime::format(&page.date, "2006-01-02T15:04:05Z07:00");

        let all_translations = translations_of(config, bundle);
        let tags: Vec<TermRef> = page.fm.tags.iter().map(|t| term_ref("tags", t)).collect();
        let categories: Vec<TermRef> = page
            .fm
            .categories
            .iter()
            .map(|c| term_ref("categories", c))
            .collect();

        let cover_abs = rendered
            .first_image_src
            .as_ref()
            .map(|src| format!("{}{src}", config.base_url));
        let fallback_img = config
            .images
            .first()
            .map(|img| format!("{}/{img}", config.base_url));
        let jsonld = schema_blogposting_json(
            &page.fm.title,
            &description,
            cover_abs.as_deref().or(fallback_img.as_deref()),
            &page.fm.tags,
            &permalink,
            &date_rfc,
            &config.author,
            about_permalink.as_deref(),
            &lang.locale,
        );
        let internal_meta = internal_meta_block(&InternalMeta {
            permalink: &permalink,
            site_title: &lang.title,
            title: &page.fm.title,
            description: &description,
            locale: &lang.locale.replace('-', "_"),
            og_type: "article",
            section: Some("posts"),
            date_published: &date_rfc,
            date_modified: &date_rfc,
            tags: &page.fm.tags,
            image: fallback_img.as_deref().unwrap_or_default(),
            word_count: Some(word_count),
            jsonld: Some(&jsonld),
        });

        posts.push(PageCtx {
            kind: "post".into(),
            is_home: false,
            is_post: true,
            layout: String::new(),
            section: "posts".into(),
            title: page.fm.title.clone(),
            description_meta: collapse_ws(&description),
            description,
            rel_permalink: rel,
            permalink,
            date_iso: gotime::format(&page.date, "2006-01-02"),
            date_display: gotime::format(&page.date, &date_layout),
            reading_time,
            tags,
            categories,
            spoiler: page.fm.spoiler.clone(),
            content_html: rendered.html,
            toc_html: markdown::toc_html(&rendered.toc),
            has_toc,
            prev: post_refs.get(idx + 1).cloned(),
            next: if idx > 0 {
                post_refs.get(idx - 1).cloned()
            } else {
                None
            },
            all_translations,
            internal_meta,
        });
    }

    let newest_date = items.first().map(|(_, p)| p.date);
    Ok(LangData {
        posts,
        post_refs,
        timeline,
        newest_date,
    })
}

fn home_ctx(
    config: &SiteConfig,
    lang: &crate::config::Language,
    data: &LangData,
    _content: &Content,
) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = if prefix.is_empty() {
        "/".to_string()
    } else {
        format!("{prefix}/")
    };
    let permalink = format!("{}{rel}", config.base_url);
    let fallback_img = config
        .images
        .first()
        .map(|img| format!("{}/{img}", config.base_url))
        .unwrap_or_default();
    let date_rfc = data
        .newest_date
        .map(|d| gotime::format(&d, "2006-01-02T15:04:05Z07:00"))
        .unwrap_or_default();

    // 首页的语言切换目标：另一语言的首页
    let all_translations = config
        .languages
        .iter()
        .map(|l| {
            let p = l.url_prefix(&config.default_lang);
            let r = if p.is_empty() {
                "/".to_string()
            } else {
                format!("{p}/")
            };
            TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            }
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta {
        permalink: &permalink,
        site_title: &lang.title,
        title: &lang.title,
        description: &lang.description,
        locale: &lang.locale.replace('-', "_"),
        og_type: "website",
        section: None,
        date_published: &date_rfc,
        date_modified: &date_rfc,
        tags: &[],
        image: &fallback_img,
        word_count: None,
        jsonld: None,
    });

    Ok(PageCtx {
        kind: "home".into(),
        is_home: true,
        title: lang.title.clone(),
        description_meta: collapse_ws(&lang.description),
        description: lang.description.clone(),
        rel_permalink: rel,
        permalink,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

fn translations_of(config: &SiteConfig, bundle: &PostBundle) -> Vec<TranslationRef> {
    // 按语言权重排序（zh 在前），包含自身
    let mut out = Vec::new();
    for lang in &config.languages {
        if bundle.versions.iter().any(|v| v.lang == lang.code) {
            let prefix = lang.url_prefix(&config.default_lang);
            let rel = format!("{prefix}/posts/{}/", content::encode_path(&bundle.slug));
            out.push(TranslationRef {
                lang: lang.code.clone(),
                locale: lang.locale.clone(),
                permalink: format!("{}{rel}", config.base_url),
                rel,
            });
        }
    }
    out
}

struct InternalMeta<'a> {
    permalink: &'a str,
    site_title: &'a str,
    title: &'a str,
    description: &'a str,
    locale: &'a str,
    og_type: &'a str,
    section: Option<&'a str>,
    date_published: &'a str,
    date_modified: &'a str,
    tags: &'a [String],
    image: &'a str,
    word_count: Option<usize>,
    jsonld: Option<&'a str>,
}

/// 复刻 Hugo 内置 opengraph/twitter_cards/schema 模板的整块输出（含缩进与空行痕迹）。
fn internal_meta_block(m: &InternalMeta) -> String {
    let esc = escape_attr;
    let mut s = String::new();
    // opengraph
    s.push_str(&format!(
        "<meta property=\"og:url\" content=\"{}\">\n",
        esc(m.permalink)
    ));
    s.push_str(&format!(
        "\t<meta property=\"og:site_name\" content=\"{}\">\n",
        esc(m.site_title)
    ));
    s.push_str(&format!(
        "\t<meta property=\"og:title\" content=\"{}\">\n",
        esc(m.title)
    ));
    s.push_str(&format!(
        "\t<meta property=\"og:description\" content=\"{}\">\n",
        esc(m.description)
    ));
    s.push_str(&format!(
        "\t<meta property=\"og:locale\" content=\"{}\">\n",
        esc(m.locale)
    ));
    s.push_str(&format!(
        "\t<meta property=\"og:type\" content=\"{}\">\n",
        m.og_type
    ));
    if m.og_type == "article" {
        if let Some(section) = m.section {
            s.push_str(&format!(
                "\t\t<meta property=\"article:section\" content=\"{}\">\n",
                esc(section)
            ));
        }
        s.push_str(&format!(
            "\t\t<meta property=\"article:published_time\" content=\"{}\">\n",
            m.date_published
        ));
        s.push_str(&format!(
            "\t\t<meta property=\"article:modified_time\" content=\"{}\">\n",
            m.date_modified
        ));
        for tag in m.tags {
            s.push_str(&format!(
                "\t\t<meta property=\"article:tag\" content=\"{}\">\n",
                esc(tag)
            ));
        }
    }
    s.push_str(&format!(
        "\t\t<meta property=\"og:image\" content=\"{}\">\n",
        esc(m.image)
    ));
    s.push_str("\n  ");
    // twitter cards
    s.push_str("\n\t<meta name=\"twitter:card\" content=\"summary_large_image\">\n");
    s.push_str(&format!(
        "\t<meta name=\"twitter:image\" content=\"{}\">\n",
        esc(m.image)
    ));
    s.push_str(&format!(
        "\t<meta name=\"twitter:title\" content=\"{}\">\n",
        esc(m.title)
    ));
    s.push_str(&format!(
        "\t<meta name=\"twitter:description\" content=\"{}\">\n",
        esc(m.description)
    ));
    s.push_str("\n  ");
    // schema
    s.push_str(&format!(
        "\n\t<meta itemprop=\"name\" content=\"{}\">\n",
        esc(m.title)
    ));
    s.push_str(&format!(
        "\t<meta itemprop=\"description\" content=\"{}\">\n",
        esc(m.description)
    ));
    s.push_str(&format!(
        "\t<meta itemprop=\"datePublished\" content=\"{}\">\n",
        m.date_published
    ));
    s.push_str(&format!(
        "\t<meta itemprop=\"dateModified\" content=\"{}\">\n",
        m.date_modified
    ));
    if let Some(wc) = m.word_count {
        s.push_str(&format!(
            "\t<meta itemprop=\"wordCount\" content=\"{wc}\">\n"
        ));
    }
    s.push_str(&format!(
        "\t<meta itemprop=\"image\" content=\"{}\">",
        esc(m.image)
    ));
    if !m.tags.is_empty() {
        s.push_str(&format!(
            "\n\t<meta itemprop=\"keywords\" content=\"{}\">",
            esc(&m.tags.join(","))
        ));
    }
    s.push_str("\n  ");
    if let Some(jsonld) = m.jsonld {
        s.push_str(&format!(
            "<script type=\"application/ld+json\">{jsonld}</script>\n"
        ));
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn schema_blogposting_json(
    title: &str,
    description: &str,
    image: Option<&str>,
    tags: &[String],
    permalink: &str,
    date_rfc: &str,
    author: &str,
    about_permalink: Option<&str>,
    locale: &str,
) -> String {
    use serde_json::{json, Map, Value};
    let mut author_obj = Map::new();
    author_obj.insert("@type".into(), json!("Person"));
    author_obj.insert("name".into(), json!(author));
    if let Some(url) = about_permalink {
        author_obj.insert("url".into(), json!(url));
    }
    // 键按字母序排列（Hugo jsonify 的行为）
    let mut root = Map::new();
    root.insert("@context".into(), json!("https://schema.org"));
    root.insert("@type".into(), json!("BlogPosting"));
    root.insert("author".into(), Value::Object(author_obj));
    root.insert("dateModified".into(), json!(date_rfc));
    root.insert("datePublished".into(), json!(date_rfc));
    root.insert("description".into(), json!(collapse_ws(description)));
    root.insert("headline".into(), json!(title));
    root.insert(
        "image".into(),
        json!(image.map(|i| vec![i.to_string()]).unwrap_or_default()),
    );
    root.insert("inLanguage".into(), json!(locale));
    root.insert("keywords".into(), json!(tags.join(", ")));
    root.insert(
        "mainEntityOfPage".into(),
        json!({"@id": permalink, "@type": "WebPage"}),
    );
    root.insert(
        "publisher".into(),
        json!({"@type": "Person", "name": author}),
    );
    serde_json::to_string(&Value::Object(root)).unwrap_or_default()
}

fn alias_html(locale: &str, target: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"{locale}\">\n\t<head>\n\t\t<title>{target}</title>\n\t\t<link rel=\"canonical\" href=\"{target}\">\n\t\t<meta charset=\"utf-8\">\n\t\t<meta http-equiv=\"refresh\" content=\"0; url={target}\">\n\t</head>\n</html>\n"
    )
}

fn render_to(
    env: &Environment,
    template: &str,
    dest: &Path,
    out_rel: &str,
    ctx: Value,
) -> Result<()> {
    let tmpl = env
        .get_template(template)
        .with_context(|| format!("加载模板 {template} 失败"))?;
    let html = tmpl
        .render(ctx)
        .with_context(|| format!("渲染 {out_rel} 失败"))?;
    write_file(dest, out_rel, &html)
}

fn write_file(dest: &Path, rel: &str, content: &str) -> Result<()> {
    let path = dest.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content).with_context(|| format!("写入 {} 失败", path.display()))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
}
