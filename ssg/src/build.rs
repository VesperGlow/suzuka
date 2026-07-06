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
    /// front matter 里的原始大小写（pagefind 的 data-pagefind-meta="tags" 用
    /// 的是 .Params.tags 原文，不是 taxonomy term 的自动 .Title）
    raw: String,
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

/// archives 页时间线视图专用（跟 YearGroup 分开：那个是侧栏的精简小组件，
/// 这个要给 archive-timeline-item 用，需要完整的 PostCard 数据）
#[derive(Serialize)]
struct YearCards {
    year: String,
    cards: Vec<PostCard>,
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
    /// `<link rel="canonical">` 目标：默认等于 permalink，front matter 声明
    /// 了 redirectTo 时改成那个绝对地址（页面本身仍然照常渲染整页内容）
    canonical: String,
    /// redirectTo 存在时的 meta-refresh + JS 跳转整块（Rust 侧构造，跟
    /// internal_meta 一样按 `| safe` 整块插入；没有 redirectTo 就是空串）
    redirect_block: String,
    /// 这个页面自己的 RSS 输出地址（home/archives/分类/标签/posts 列表页才有，
    /// 单篇文章、about/guestbook、404 没有）
    rss_rel: Option<String>,
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

/// 归档卡片/列表卡片（archive-card.html、post-card.html）共用的单篇文章数据，
/// 也是 RSS item 的数据来源
#[derive(Serialize, Clone)]
struct PostCard {
    title: String,
    rel: String,
    permalink: String,
    date_iso: String,
    date_display: String,
    date_mmdd: String,
    year: String,
    /// RSS pubDate：`Mon, 02 Jan 2006 15:04:05 -0700`
    pub_date: String,
    word_count: usize,
    image_count: usize,
    summary: String,
    /// data-categories / data-tags 属性用的原始大小写 JSON 数组
    categories_raw: Vec<String>,
    tags_raw: Vec<String>,
    /// 上面两个数组序列化成 JSON 并做好 HTML 属性转义，模板里直接
    /// `| safe` 插进 data-categories/data-tags；不能用 minijinja 的
    /// `tojson` 过滤器，它会把结果标成"已安全"从而跳过属性转义
    categories_attr: String,
    tags_attr: String,
    /// RSS `<category>`：taxonomy term 的自动 .Title（首字母大写），不是原始大小写
    tags_title: Vec<String>,
    cover_url: Option<String>,
    content_html: String,
    /// 纯文本正文（已合并空白），index.json 的 content 字段用
    plain: String,
    /// date_published/date_modified 用的 RFC3339，`Z07:00` 记法（有别于
    /// internal_meta 用的固定 `-07:00`）
    date_rfc3339: String,
    image: Option<String>,
    /// 排序/跟 about、guestbook 混排用；不需要序列化给模板
    #[serde(skip)]
    date: DateTime<FixedOffset>,
}

/// 一个 taxonomy term（单个分类/标签）聚合出的页面数据
#[derive(Serialize, Clone)]
struct TermAgg {
    raw: String,
    title: String,
    slug: String,
    rel: String,
    cards: Vec<PostCard>,
}

/// 按 rel 找到（或首次创建）一个 term 聚合项；Vec 保留首次出现的顺序
fn term_agg<'a>(terms: &'a mut Vec<TermAgg>, raw: &str, tref: &TermRef) -> &'a mut TermAgg {
    if let Some(pos) = terms.iter().position(|t| t.rel == tref.rel) {
        &mut terms[pos]
    } else {
        terms.push(TermAgg {
            raw: raw.to_string(),
            title: tref.title.clone(),
            slug: content::urlize(raw),
            rel: tref.rel.clone(),
            cards: Vec::new(),
        });
        terms.last_mut().unwrap()
    }
}

/// Hugo `site.Taxonomies.<x>.ByCount`：按数量倒序，同数量按标题字母序
/// （不区分大小写）打平
fn sort_terms_by_count(terms: &mut [TermAgg]) {
    terms.sort_by(|a, b| {
        b.cards
            .len()
            .cmp(&a.cards.len())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

struct LangData {
    posts: Vec<PageCtx>,
    post_refs: Vec<PostRef>,
    timeline: Vec<YearGroup>,
    newest_date: Option<DateTime<FixedOffset>>,
    total_words: usize,
    categories: Vec<TermAgg>,
    tags: Vec<TermAgg>,
    cards: Vec<PostCard>,
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
    env.add_filter("urlquery", |s: String| {
        // Hugo 的 html/template 对 href 属性里的 URL 值会把 '+' 进一步转成
        // 数字实体 &#43;（其余已经 %XX 百分号编码的字符不受影响），照抄这个怪癖
        content::query_escape(&s).replace('+', "&#43;")
    });

    // 每种语言的独立页 URL（sidebar / guestbook 链接用）
    // front matter 的 url 是最终绝对路径本身（英文页已经自带 /en 前缀），
    // 不需要再叠一层语言前缀
    let special_rel = |key: &str, lang: &str| -> String {
        content
            .pages
            .iter()
            .find(|p| p.key == key && p.lang == lang)
            .and_then(|p| p.fm.url.clone())
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

        // 首页的 RSS / JSON Feed / index.json / guestbook-posts.json
        // （Hugo `[outputs] home = [HTML, RSS, JSON, GuestbookPosts, JSONFeed]`）
        {
            let home_permalink = format!("{}{}", config.base_url, site.home_rel);
            let card_refs: Vec<&PostCard> = data.cards.iter().collect();

            // 首页 RSS 用全站 RegularPages（文章 + about/guestbook），不只是 posts
            let mut rss_items: Vec<OwnedRssItem> = data
                .cards
                .iter()
                .map(|c| OwnedRssItem {
                    title: c.title.clone(),
                    link: c.permalink.clone(),
                    pub_date: c.pub_date.clone(),
                    date: c.date,
                    categories: c.tags_title.clone(),
                    content_html: c.content_html.clone(),
                })
                .collect();
            for raw_page in content
                .pages
                .iter()
                .filter(|p| p.lang == lang.code && p.kind == PageKind::Page)
            {
                let rendered = markdown::render(&raw_page.body, &[], "");
                let url = raw_page.fm.url.clone().unwrap_or_default();
                rss_items.push(OwnedRssItem {
                    title: raw_page.fm.title.clone(),
                    link: format!("{}{url}", config.base_url),
                    pub_date: gotime::format(&raw_page.date, "Mon, 02 Jan 2006 15:04:05 -0700"),
                    date: raw_page.date,
                    categories: Vec::new(),
                    content_html: rendered.html,
                });
            }
            rss_items.sort_by(|a, b| b.date.cmp(&a.date));
            let last_build_date = rss_items.first().map(|c| c.pub_date.clone());

            let rss = render_rss(
                &lang.title,
                &home_permalink,
                &lang.description,
                &lang.title,
                &lang.locale,
                &config.author,
                chrono::Utc::now().year(),
                last_build_date.as_deref(),
                &format!("{}{}", config.base_url, site.rss_rel),
                &rss_items
                    .iter()
                    .map(|c| RssItem {
                        title: &c.title,
                        link: &c.link,
                        pub_date: &c.pub_date,
                        categories: &c.categories,
                        content_html: Some(&c.content_html),
                    })
                    .collect::<Vec<_>>(),
                &config.base_url,
            );
            write_file(
                dest,
                format!("{prefix}/feed.xml").trim_start_matches('/'),
                &rss,
            )?;

            write_file(
                dest,
                format!("{prefix}/feed.json").trim_start_matches('/'),
                &feed_json(
                    &lang.title,
                    &home_permalink,
                    &format!("{}{}", config.base_url, site.jsonfeed_rel),
                    &lang.description,
                    &lang.locale,
                    &config.author,
                    &card_refs,
                    &config.base_url,
                ),
            )?;

            write_file(
                dest,
                format!("{prefix}/index.json").trim_start_matches('/'),
                &index_json(&card_refs),
            )?;

            let default_bundle_cards: Vec<GuestbookItem> = content
                .posts
                .iter()
                .map(|bundle| {
                    let version = bundle
                        .versions
                        .iter()
                        .find(|v| v.lang == lang.code)
                        .or_else(|| {
                            bundle
                                .versions
                                .iter()
                                .find(|v| v.lang == config.default_lang)
                        })
                        .expect("bundle 至少有一个语言版本");
                    let rel = format!("{prefix}/posts/{}/", content::encode_path(&bundle.slug));
                    GuestbookItem {
                        title: version.fm.title.clone(),
                        rel,
                        date_iso: gotime::format(&version.date, "2006-01-02"),
                    }
                })
                .collect();
            write_file(
                dest,
                format!("{prefix}/guestbook-posts.json").trim_start_matches('/'),
                &guestbook_posts_json(&default_bundle_cards),
            )?;
        }

        // 404 页
        let notfound = notfound_ctx(&config, lang)?;
        let notfound_out = format!("{prefix}/404.html").trim_start_matches('/').to_string();
        render_to(
            &env,
            "notfound.html",
            dest,
            &notfound_out,
            minijinja::context! {
                lang => lang.code.clone(),
                site => &site,
                page => &notfound,
                posts => &data.post_refs,
                timeline => &data.timeline,
                theme_js => &assets.theme_js,
            },
        )?;

        // 独立页面：about / guestbook（跳过 _index 之类的 Section 页，
        // 它们只是 Hugo 端 `build.render: never` 的列表页，没有真实 url）
        for raw_page in content
            .pages
            .iter()
            .filter(|p| p.lang == lang.code && p.kind == PageKind::Page)
        {
            let rendered = markdown::render(&raw_page.body, &[], "");
            let word_count = if lang.has_cjk {
                content::word_count_cjk(&rendered.plain)
            } else {
                content::word_count_latin(&rendered.plain)
            };
            let ctx = static_page_ctx(&config, lang, &content, raw_page, rendered.html, word_count)?;
            let template = match raw_page.fm.layout.as_deref() {
                Some("guestbook") => "guestbook.html",
                _ => "about.html",
            };
            let out_rel = format!("{}index.html", ctx.rel_permalink.trim_start_matches('/'));
            render_to(
                &env,
                template,
                dest,
                &out_rel,
                minijinja::context! {
                    lang => lang.code.clone(),
                    site => &site,
                    page => &ctx,
                    posts => &data.post_refs,
                    timeline => &data.timeline,
                    theme_js => &assets.theme_js,
                    post_count_fmt => format_number(data.post_refs.len()),
                    total_words_fmt => format_number(data.total_words),
                },
            )?;
        }

        // archives 页 + 它自己的 feed.xml（items 用 archives 页自己的
        // RegularPages，本站底下没有内容文件，所以永远是空 feed）
        if let Some(raw_page) = content.archives.iter().find(|p| p.lang == lang.code) {
            let rendered = markdown::render(&raw_page.body, &[], "");
            let ctx = archives_ctx(&config, lang, &content, raw_page, rendered.html)?;
            let out_rel = format!("{}index.html", ctx.rel_permalink.trim_start_matches('/'));

            let mut archive_years: Vec<YearCards> = Vec::new();
            for card in &data.cards {
                match archive_years.last_mut() {
                    Some(group) if group.year == card.year => group.cards.push(card.clone()),
                    _ => archive_years.push(YearCards {
                        year: card.year.clone(),
                        cards: vec![card.clone()],
                    }),
                }
            }
            let mut categories_by_count = data.categories.clone();
            sort_terms_by_count(&mut categories_by_count);
            let mut tags_by_count = data.tags.clone();
            sort_terms_by_count(&mut tags_by_count);

            render_to(
                &env,
                "archives.html",
                dest,
                &out_rel,
                minijinja::context! {
                    lang => lang.code.clone(),
                    site => &site,
                    page => &ctx,
                    posts => &data.post_refs,
                    timeline => &data.timeline,
                    theme_js => &assets.theme_js,
                    archive_cards => &data.cards,
                    archive_years => &archive_years,
                    categories_by_count => &categories_by_count,
                    tags_by_count => &tags_by_count,
                    total_posts => data.cards.len(),
                    all_active => true,
                    active_category_rel => Option::<String>::None,
                },
            )?;

            let rss = render_rss(
                &channel_title(&ctx.title, &lang.title),
                &ctx.permalink,
                &ctx.description,
                &lang.title,
                &lang.locale,
                &config.author,
                chrono::Utc::now().year(),
                None,
                &format!("{}{}", config.base_url, ctx.rss_rel.clone().unwrap_or_default()),
                &[],
                &config.base_url,
            );
            write_file(
                dest,
                format!("{prefix}/archives/feed.xml").trim_start_matches('/'),
                &rss,
            )?;
        }

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
    write_file(
        dest,
        "robots.txt",
        &format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
            config.base_url
        ),
    )?;

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
        title: content::hugo_title_case(name),
        raw: name.to_string(),
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
    let mut total_words = 0usize;
    // 用 Vec 保留「首次出现」顺序，而不是 HashMap（迭代顺序每次进程随机，
    // 会导致构建产物不确定）
    let mut cat_terms: Vec<TermAgg> = Vec::new();
    let mut tag_terms: Vec<TermAgg> = Vec::new();
    let mut all_cards: Vec<PostCard> = Vec::new();
    for (idx, (bundle, page)) in items.iter().enumerate() {
        let rel = format!("{prefix}/posts/{}/", content::encode_path(&bundle.slug));
        let bundle_rel = format!("/posts/{}/", content::encode_path(&bundle.slug));
        let rendered = markdown::render(&page.body, &bundle.resources, &bundle_rel);
        let word_count = if lang.has_cjk {
            content::word_count_cjk(&rendered.plain)
        } else {
            content::word_count_latin(&rendered.plain)
        };
        total_words += word_count;
        let reading_time = if lang.has_cjk {
            (word_count + 500) / 501
        } else {
            (word_count + 212) / 213
        };
        let h2_count = rendered.toc.iter().filter(|t| t.level == 2).count();
        let has_toc = h2_count > 1;
        let permalink = format!("{}{rel}", config.base_url);
        let description = page.fm.description.clone().unwrap_or_default();
        let date_rfc = gotime::format(&page.date, "2006-01-02T15:04:05-07:00");

        let all_translations = translations_of(config, bundle);
        let tags: Vec<TermRef> = page.fm.tags.iter().map(|t| term_ref("tags", t)).collect();
        let categories: Vec<TermRef> = page
            .fm
            .categories
            .iter()
            .map(|c| term_ref("categories", c))
            .collect();
        // article:tag / jsonld keywords 用的是 taxonomy term 的自动 .Title（每词首字母大写），
        // 不是 front matter 里的原始大小写
        let og_tags: Vec<String> = page
            .fm
            .tags
            .iter()
            .map(|t| content::hugo_title_case(t))
            .collect();

        // 正文里出现的第一张图（不管 front matter 有没有声明），只给 jsonld 的兜底用
        let cover_abs = rendered
            .first_image_src
            .as_ref()
            .map(|src| format!("{}{src}", config.base_url));
        let fallback_img = config
            .images
            .first()
            .map(|img| format!("{}/{img}", config.base_url));

        // og:image / twitter:image / itemprop="image"：复刻 Hugo 内置
        // _funcs/get-page-images.html —— 只认 front matter images（按 bundle
        // 资源解析出正确的 /posts/slug/ 前缀），没声明就退到站点默认图。
        // 不看正文首图（本站没有用到 get-page-images 的 cover/feature/thumbnail
        // 那一档兜底，故未实现）。
        let og_image = page
            .fm
            .images
            .first()
            .map(|img| resolve_bundle_image(config, bundle, &bundle_rel, img))
            .or_else(|| fallback_img.clone());

        // jsonld 的 image：复刻本项目自己的 schema-blogposting.html —— front
        // matter images 直接绝对化、不经过 bundle 解析（Hugo 那份模板本身没调
        // Resources.GetMatch，是个「忠实复刻」的怪癖）；没声明才退到正文首图
        // （这一步在 archive-cover-url.html 里是有 Resources.GetMatch 的，
        // 所以 cover_abs 的 /posts/slug/ 前缀是对的），最后才是站点默认图。
        let jsonld_image = match page.fm.images.first() {
            Some(img) => Some(format!("{}/{img}", config.base_url)),
            None => cover_abs.clone().or_else(|| fallback_img.clone()),
        };

        let jsonld = schema_blogposting_json(
            &page.fm.title,
            &description,
            jsonld_image.as_deref(),
            &og_tags,
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
            tags: &og_tags,
            image: og_image.as_deref().unwrap_or_default(),
            word_count: Some(word_count),
            jsonld: Some(&jsonld),
        });

        // archive-cover-url.html 只看 Params.cover/image/featured_image
        // （本站都没用），从不看 front matter 的 images——所以归档卡片封面
        // 永远是正文里第一张图，跟 og:image 的解析逻辑（会优先用 front
        // matter images）是两条不同的路径，故意不共用
        let cover_url = rendered.first_image_src.clone();
        let card = PostCard {
            title: page.fm.title.clone(),
            rel: rel.clone(),
            permalink: permalink.clone(),
            date_iso: post_refs[idx].date_iso.clone(),
            date_display: post_refs[idx].date_display.clone(),
            date_mmdd: post_refs[idx].date_mmdd.clone(),
            year: post_refs[idx].year.clone(),
            pub_date: gotime::format(&page.date, "Mon, 02 Jan 2006 15:04:05 -0700"),
            word_count,
            image_count: rendered.html.matches("<img").count(),
            summary: description.clone(),
            categories_attr: json_attr(&page.fm.categories),
            tags_attr: json_attr(&page.fm.tags),
            categories_raw: page.fm.categories.clone(),
            tags_raw: page.fm.tags.clone(),
            tags_title: og_tags.clone(),
            cover_url,
            content_html: rendered.html.clone(),
            plain: collapse_ws(&html_unescape_typographic(&rendered.plain)),
            date_rfc3339: gotime::format(&page.date, "2006-01-02T15:04:05Z07:00"),
            // JSON Feed 的 image：只认 front matter images，没有站点默认图兜底
            // （跟 og:image 不一样），绝对化但不经过 bundle 解析
            image: page
                .fm
                .images
                .first()
                .map(|img| format!("{}/{img}", config.base_url)),
            date: page.date,
        };
        for cat in &page.fm.categories {
            let tref = term_ref("categories", cat);
            term_agg(&mut cat_terms, cat, &tref).cards.push(card.clone());
        }
        for tag in &page.fm.tags {
            let tref = term_ref("tags", tag);
            term_agg(&mut tag_terms, tag, &tref).cards.push(card.clone());
        }
        all_cards.push(card);

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
            canonical: permalink.clone(),
            redirect_block: String::new(),
            rss_rel: None,
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
        total_words,
        categories: cat_terms,
        tags: tag_terms,
        cards: all_cards,
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
        .map(|d| gotime::format(&d, "2006-01-02T15:04:05-07:00"))
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
        canonical: permalink.clone(),
        rss_rel: Some(format!("{prefix}/feed.xml")),
        permalink,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

/// 独立页面（about/guestbook）：og:type 固定是 article、section 固定是
/// pages（跟 posts 的 section="posts" 对应），跟文章页共用同一套
/// internal_meta_block，但没有 jsonld、没有 tags。
fn static_page_ctx(
    config: &SiteConfig,
    lang: &crate::config::Language,
    content: &Content,
    page: &RawPage,
    content_html: String,
    word_count: usize,
) -> Result<PageCtx> {
    let url = page.fm.url.clone().unwrap_or_default();
    let permalink = format!("{}{url}", config.base_url);
    let description = page.fm.description.clone().unwrap_or_default();
    let date_rfc = gotime::format(&page.date, "2006-01-02T15:04:05-07:00");
    let fallback_img = config
        .images
        .first()
        .map(|img| format!("{}/{img}", config.base_url))
        .unwrap_or_default();

    let all_translations = config
        .languages
        .iter()
        .filter_map(|l| {
            content
                .pages
                .iter()
                .find(|p| p.key == page.key && p.lang == l.code)
                .and_then(|p| p.fm.url.clone())
                .map(|u| TranslationRef {
                    lang: l.code.clone(),
                    locale: l.locale.clone(),
                    permalink: format!("{}{u}", config.base_url),
                    rel: u,
                })
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta {
        permalink: &permalink,
        site_title: &lang.title,
        title: &page.fm.title,
        description: &description,
        locale: &lang.locale.replace('-', "_"),
        og_type: "article",
        section: Some("pages"),
        date_published: &date_rfc,
        date_modified: &date_rfc,
        tags: &[],
        image: &fallback_img,
        word_count: Some(word_count),
        jsonld: None,
    });

    let (canonical, redirect_block) = match redirect_fields(config, page.fm.redirect_to.as_deref())
    {
        Some((c, b)) => (c, b),
        None => (permalink.clone(), String::new()),
    };

    Ok(PageCtx {
        kind: page.key.clone(),
        layout: page.fm.layout.clone().unwrap_or_default(),
        section: "pages".into(),
        title: page.fm.title.clone(),
        description_meta: collapse_ws(&description),
        description,
        rel_permalink: url,
        canonical,
        redirect_block,
        permalink,
        content_html,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

/// archives 页：og:type 固定 website（跟 home 一样，section 页而非 article），
/// 没有 jsonld、没有 tags；URL 是算出来的（content/archives/_index.md 没有
/// 自己的 url 字段，不像 about/guestbook）
fn archives_ctx(
    config: &SiteConfig,
    lang: &crate::config::Language,
    content: &Content,
    page: &RawPage,
    content_html: String,
) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = format!("{prefix}/archives/");
    let permalink = format!("{}{rel}", config.base_url);
    let description = page.fm.description.clone().unwrap_or_default();
    let date_rfc = if page.date.timestamp() == 0 {
        String::new()
    } else {
        gotime::format(&page.date, "2006-01-02T15:04:05-07:00")
    };
    let fallback_img = config
        .images
        .first()
        .map(|img| format!("{}/{img}", config.base_url))
        .unwrap_or_default();

    let all_translations = config
        .languages
        .iter()
        .filter_map(|l| {
            content.archives.iter().find(|p| p.lang == l.code)?;
            let p = l.url_prefix(&config.default_lang);
            let r = format!("{p}/archives/");
            Some(TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            })
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta {
        permalink: &permalink,
        site_title: &lang.title,
        title: &page.fm.title,
        description: &description,
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

    let (canonical, redirect_block) = match redirect_fields(config, page.fm.redirect_to.as_deref())
    {
        Some((c, b)) => (c, b),
        None => (permalink.clone(), String::new()),
    };

    Ok(PageCtx {
        kind: "archives".into(),
        layout: "archives".into(),
        section: "archives".into(),
        title: page.fm.title.clone(),
        description_meta: collapse_ws(&description),
        description,
        rel_permalink: rel,
        canonical,
        redirect_block,
        rss_rel: Some(format!("{prefix}/archives/feed.xml")),
        permalink,
        content_html,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

fn notfound_ctx(config: &SiteConfig, lang: &crate::config::Language) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = format!("{prefix}/404.html");
    let permalink = format!("{}{rel}", config.base_url);

    let all_translations = config
        .languages
        .iter()
        .map(|l| {
            let p = l.url_prefix(&config.default_lang);
            let r = format!("{p}/404.html");
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
        // Hugo 对没有专属 content 文件的内置 404 page kind 用这个固定标题
        title: "404 Page not found",
        description: &lang.description,
        locale: &lang.locale.replace('-', "_"),
        og_type: "website",
        section: None,
        date_published: "",
        date_modified: "",
        tags: &[],
        image: &config
            .images
            .first()
            .map(|img| format!("{}/{img}", config.base_url))
            .unwrap_or_default(),
        word_count: None,
        jsonld: None,
    });

    Ok(PageCtx {
        kind: "notfound".into(),
        title: "404 Page not found".into(),
        description_meta: collapse_ws(&lang.description),
        description: lang.description.clone(),
        rel_permalink: rel,
        canonical: permalink.clone(),
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
    if !m.date_published.is_empty() {
        s.push_str(&format!(
            "\t<meta itemprop=\"datePublished\" content=\"{}\">\n",
            m.date_published
        ));
        s.push_str(&format!(
            "\t<meta itemprop=\"dateModified\" content=\"{}\">\n",
            m.date_modified
        ));
    }
    if let Some(wc) = m.word_count {
        if wc > 0 {
            s.push_str(&format!(
                "\t<meta itemprop=\"wordCount\" content=\"{wc}\">\n"
            ));
        }
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

/// 对应 Hugo `lang.FormatNumberCustom 0 n`：千位加英文逗号分隔，不带小数
fn format_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Hugo `htmlUnescape` 只在 index.json 的 `.Plain` 上用了一次；实测行为是
/// 排版实体（弯引号/破折号/省略号，都是"配对成功"才产生的）会解码回真实字符，
/// 但 typographer 遇到无法判断开合的直引号、原样转义成的 `&quot;`/`&#34;`
/// 反而不会被解开——所以这里故意不处理 quot/#34/#39/apos，让它保持实体文本，
/// 跟 diff.rs 里 `&#34;`/`&quot;` 的归一化规则对上
fn html_unescape_typographic(s: &str) -> String {
    s.replace("&ldquo;", "\u{201c}")
        .replace("&rdquo;", "\u{201d}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&rsquo;", "\u{2019}")
        .replace("&hellip;", "\u{2026}")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 把 front matter `images` 里的文件名解析成完整 URL：命中 bundle 内资源就
/// 带上 /posts/slug/ 前缀，否则按 Hugo 的 else 分支当站点根路径处理
fn resolve_bundle_image(config: &SiteConfig, bundle: &PostBundle, bundle_rel: &str, img: &str) -> String {
    if bundle.resources.iter().any(|r| r.name == *img) {
        format!("{}{bundle_rel}{img}", config.base_url)
    } else {
        format!("{}/{img}", config.base_url)
    }
}

/// redirectTo 存在时算出 (canonical 绝对地址, meta-refresh+JS 跳转整块)；
/// 复刻 baseof.html 里 canonical 用 absURL、meta/script 用 relURL 的区别——
/// redirect_to 本身已经是带语言前缀的根相对路径，relURL 等价于原样返回
fn redirect_fields(config: &SiteConfig, redirect_to: Option<&str>) -> Option<(String, String)> {
    let target = redirect_to?;
    let abs = format!("{}{target}", config.base_url);
    let js = serde_json::to_string(target).unwrap_or_default();
    let block = format!(
        "<meta http-equiv=\"refresh\" content=\"0; url={}\">\n  <script>window.location.replace({js});</script>\n  ",
        escape_attr(target)
    );
    Some((abs, block))
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#x27;")
}

/// `data-categories`/`data-tags` 属性：`{{ $slice | jsonify }}` 落进一个
/// HTML 属性值，要走属性转义，不能像 `<script>` 里那样原样插入
fn json_attr(items: &[String]) -> String {
    escape_attr(&serde_json::to_string(items).unwrap_or_default())
}

/// RSS item：标题/链接/标签是普通文章卡片
struct RssItem<'a> {
    title: &'a str,
    link: &'a str,
    pub_date: &'a str,
    /// RSS `<category>`；文章条目用 taxonomy term 的 .Title，term/section
    /// 列表条目（feed 里列的是子 term 页而不是文章）没有分类，传空切片
    categories: &'a [String],
    /// 完整正文（会话把站内根相对 src/href 改成绝对地址后逐字节 HTML 转义）；
    /// term/section 列表条目没有正文，传空串
    content_html: Option<&'a str>,
}

/// 首页 RSS 的条目是全站 RegularPages（文章 + about/guestbook 这类独立页），
/// 不像 feed.json/index.json 那样只看 posts —— 所以单独存一份带日期的
/// 拥有所有权的列表，跟文章卡片按日期混排后再借出 RssItem
struct OwnedRssItem {
    title: String,
    link: String,
    pub_date: String,
    date: DateTime<FixedOffset>,
    categories: Vec<String>,
    content_html: String,
}

/// 复刻 Hugo 内置 `_default/rss.xml`：Hugo 的非 HTML 输出格式走 text/template
/// （不自动转义），模板里没写 `| html` 的字段（title/link/description/
/// category）原样插入，只有 item 的 `<description>` 显式转义
#[allow(clippy::too_many_arguments)]
fn render_rss(
    channel_title: &str,
    channel_link: &str,
    channel_description: &str,
    site_title: &str,
    locale: &str,
    author: &str,
    current_year: i32,
    last_build_date: Option<&str>,
    self_link: &str,
    items: &[RssItem],
    base_url: &str,
) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    s.push_str("  <channel>\n");
    s.push_str(&format!("    <title>{}</title>\n", escape_attr(channel_title)));
    s.push_str(&format!("    <link>{channel_link}</link>\n"));
    s.push_str(&format!(
        "    <description>{}</description>\n",
        escape_attr(channel_description)
    ));
    s.push_str(&format!(
        "    <generator>{}</generator>\n",
        escape_attr(site_title)
    ));
    s.push_str(&format!("    <language>{locale}</language>\n"));
    s.push_str(&format!("    <managingEditor>{author}</managingEditor>\n"));
    s.push_str(&format!("    <webMaster>{author}</webMaster>\n"));
    s.push_str(&format!(
        "    <copyright>© {current_year} {author}</copyright>\n"
    ));
    if let Some(lbd) = last_build_date {
        s.push_str(&format!("    <lastBuildDate>{lbd}</lastBuildDate>\n"));
    }
    s.push_str(&format!(
        "    <atom:link href=\"{self_link}\" rel=\"self\" type=\"application/rss+xml\" />\n"
    ));
    for item in items {
        s.push_str("    <item>\n");
        s.push_str(&format!("      <title>{}</title>\n", escape_attr(item.title)));
        s.push_str(&format!("      <link>{}</link>\n", item.link));
        s.push_str(&format!("      <pubDate>{}</pubDate>\n", item.pub_date));
        s.push_str(&format!("      <guid>{}</guid>\n", item.link));
        for cat in item.categories {
            s.push_str(&format!("      <category>{}</category>\n", escape_attr(cat)));
        }
        let content = item
            .content_html
            .map(|html| {
                html.replace("src=\"/", &format!("src=\"{base_url}/"))
                    .replace("href=\"/", &format!("href=\"{base_url}/"))
            })
            .unwrap_or_default();
        s.push_str(&format!(
            "      <description>{}</description>\n",
            escape_attr(&content)
        ));
        s.push_str("    </item>\n");
    }
    s.push_str("  </channel>\n");
    s.push_str("</rss>\n");
    s
}

/// Hugo `{{ if eq .Title site.Title }}{{ site.Title }}{{ else }}{{ with .Title }}{{ . }} · {{ end }}{{ site.Title }}{{ end }}`
fn channel_title(page_title: &str, site_title: &str) -> String {
    if page_title == site_title || page_title.is_empty() {
        site_title.to_string()
    } else {
        format!("{page_title} · {site_title}")
    }
}

/// Hugo `strings.Truncate <max>`：按字符数截断，避免切在词中间——是往前
/// 扫到下一个空白（而不是回退到上一个），所以结果常常比 max 更长；
/// 截断时补 " …"（空格 + 省略号）
fn truncate_runes(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while cut < chars.len() && !chars[cut].is_whitespace() {
        cut += 1;
    }
    let mut out: String = chars[..cut].iter().collect();
    out.push_str(" …");
    out
}

/// 一条 sitemap.xml `<url>` 记录
struct SitemapEntry {
    /// 带语言前缀的根相对路径，如 "/en/tags/galgame/"
    rel: String,
    /// lastmod：None 表示页面没有真实日期（Hugo `.Lastmod.IsZero`），不输出这一行
    lastmod: Option<String>,
    /// 去掉语言前缀后的路径，用来跟其他语言的条目配对判断是否互为翻译
    match_key: String,
}

/// 复刻 Hugo 内置 sitemap 模板：sitemapindex 汇总各语言 urlset，
/// 每语言一份 urlset，条目按 lastmod 倒序（同 lastmod 的相对顺序不追求
/// 跟 Hugo 内部排序一致——纯粹是展示顺序，不影响 SEO 语义，对拍时按
/// url 集合而非顺序比较，见 diff.rs）
fn render_sitemap_index(base_url: &str, langs: &[(&str, Option<&str>)]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\t\n");
    for (prefix, lastmod) in langs {
        s.push_str("\t\t<sitemap>\n\t\t\t<loc>");
        s.push_str(&format!("{base_url}{prefix}/sitemap.xml"));
        s.push_str("</loc>\n\t\t\t\n");
        if let Some(lm) = lastmod {
            s.push_str(&format!("\t\t\t\t<lastmod>{lm}</lastmod>\n\t\t\t\n"));
        }
        s.push_str("\t\t</sitemap>\n\t\n");
    }
    s.push_str("</sitemapindex>\n");
    s
}

/// key = match_key，value = 这个路径在各语言下的 (hreflang, rel)，按
/// config.languages 的固定顺序（zh 在前）——不管在渲染哪个语言的 urlset，
/// alternate 链接的先后顺序都固定，跟"自己是谁"无关
fn render_sitemap_urlset(
    base_url: &str,
    entries: &[SitemapEntry],
    alts_by_key: &HashMap<String, Vec<(String, String)>>,
) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"\n  xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n");
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
                        "    <xhtml:link\n                rel=\"alternate\"\n                hreflang=\"{hreflang}\"\n                href=\"{base_url}{rel}\"\n                />\n"
                    ));
                }
            }
        }
        s.push_str("  </url>");
    }
    s.push('\n');
    s.push_str("</urlset>\n");
    s
}

/// Hugo `jsonify` 对 `map[string]interface{}` 按键的字母序输出（这个 crate
/// 开了 `preserve_order`，所以要手动按字母序 insert，不能用 `json!{}` 字面量
/// 顺序，也不能事后再插可选字段）
fn ordered_json(pairs: Vec<(&str, serde_json::Value)>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    serde_json::Value::Object(map)
}

/// Go `encoding/json.Marshal` 默认把字符串里的 `<`、`>`、`&` 转成
/// `<`/`>`/`&`（防止 JSON 被当 HTML 解析时出问题），Hugo
/// `jsonify` 没有关掉这个默认行为；serde_json 不做这层转义，序列化完再
/// 全局替换一遍效果等价——这三个字符在合法 JSON 里只会出现在字符串值内部，
/// 不会是结构字符，所以整串替换是安全的
fn json_html_escape(s: &str) -> String {
    s.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// layouts/index.json：全部文章的极简摘要列表（url/title/date/tags/content）
fn index_json(items: &[&PostCard]) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|c| {
            ordered_json(vec![
                ("content", serde_json::json!(truncate_runes(&c.plain, 800))),
                ("date", serde_json::json!(c.date_display)),
                ("tags", serde_json::json!(c.tags_raw)),
                ("title", serde_json::json!(c.title)),
                ("url", serde_json::json!(c.rel)),
            ])
        })
        .collect();
    json_html_escape(
        &serde_json::to_string(&ordered_json(vec![("items", serde_json::json!(arr))]))
            .unwrap_or_default(),
    )
}

/// layouts/index.guestbookposts.json：关于页站点总览用的极简列表
fn guestbook_posts_json(items: &[GuestbookItem]) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|it| {
            ordered_json(vec![
                ("date", serde_json::json!(it.date_iso)),
                ("title", serde_json::json!(it.title)),
                ("url", serde_json::json!(it.rel)),
            ])
        })
        .collect();
    json_html_escape(
        &serde_json::to_string(&ordered_json(vec![("items", serde_json::json!(arr))]))
            .unwrap_or_default(),
    )
}

struct GuestbookItem {
    title: String,
    rel: String,
    date_iso: String,
}

/// layouts/index.jsonfeed.json（JSON Feed 1.1）
#[allow(clippy::too_many_arguments)]
fn feed_json(
    site_title: &str,
    home_url: &str,
    feed_url: &str,
    description: &str,
    locale: &str,
    author: &str,
    items: &[&PostCard],
    base_url: &str,
) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|c| {
            let content_html = c
                .content_html
                .replace("src=\"/", &format!("src=\"{base_url}/"))
                .replace("href=\"/", &format!("href=\"{base_url}/"));
            let mut pairs = vec![
                ("content_html", serde_json::json!(content_html)),
                ("date_modified", serde_json::json!(c.date_rfc3339)),
                ("date_published", serde_json::json!(c.date_rfc3339)),
                ("id", serde_json::json!(c.permalink)),
            ];
            if let Some(img) = &c.image {
                pairs.push(("image", serde_json::json!(img)));
            }
            if !c.summary.is_empty() {
                pairs.push(("summary", serde_json::json!(c.summary)));
            }
            pairs.push(("tags", serde_json::json!(c.tags_raw)));
            pairs.push(("title", serde_json::json!(c.title)));
            pairs.push(("url", serde_json::json!(c.permalink)));
            ordered_json(pairs)
        })
        .collect();
    let doc = ordered_json(vec![
        ("authors", serde_json::json!([{"name": author}])),
        ("description", serde_json::json!(description)),
        ("feed_url", serde_json::json!(feed_url)),
        ("home_page_url", serde_json::json!(home_url)),
        ("items", serde_json::json!(arr)),
        ("language", serde_json::json!(locale)),
        ("title", serde_json::json!(site_title)),
        ("version", serde_json::json!("https://jsonfeed.org/version/1.1")),
    ]);
    json_html_escape(&serde_json::to_string_pretty(&doc).unwrap_or_default())
}
