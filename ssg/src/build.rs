//! 构建编排：内容 → 页面模型 → minijinja 渲染 → 写盘。

use crate::config::{Language, SiteConfig};
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
    /// 的是原文，不是 term 的自动 title）
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
struct YearCards<'a> {
    year: String,
    cards: Vec<&'a PostCard>,
}

/// pagination-nav.html 的页码格子
#[derive(Serialize)]
struct PagerPage {
    number: usize,
    url: String,
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
    /// 语言切换按钮的兜底目标：当前页在另一语言没有对应翻译时（比如没有
    /// 对应内容的 taxonomy term 页），退回那个语言的首页——跟
    /// `page.all_translations`（决定 `<head>` 里真正的 hreflang alternate，
    /// 没有翻译就不输出）是两条独立的逻辑
    other_lang_home_rel: String,
    other_lang_locale: String,
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
    /// 阅读数/点赞与留言板引用用的规范文章路径（恒为默认语言形态
    /// `/posts/<slug>/`，不带语言前缀）：backend 只认这一形态，且中英文
    /// 版本共享同一份计数。仅文章页非空。
    counter_path: String,
    /// `<link rel="canonical">` 目标：默认等于 permalink，front matter 声明
    /// 了 redirectTo 时改成那个绝对地址（页面本身仍然照常渲染整页内容）
    canonical: String,
    /// redirectTo 存在时的 meta-refresh + JS 跳转整块（Rust 侧构造，跟
    /// internal_meta 一样按 `| safe` 整块插入；没有 redirectTo 就是空串）
    redirect_block: String,
    /// 这个页面自己的 RSS 输出地址（home / term 页 / posts 列表页才有，
    /// 单篇文章、about/guestbook、archives、404 没有）
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
    /// og/twitter/schema/JSON-LD 的整块 head 输出（Rust 侧构造）
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
    /// RSS pubDate（RFC 1123）
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
    /// RSS `<category>` / og keywords：term 的自动 title（首字母大写），不是原始大小写
    tags_title: Vec<String>,
    cover_url: Option<String>,
    /// 封面全部缩放档位（含原图）的 srcset；首图无变体时为 None，模板退回单 src
    cover_srcset: Option<String>,
    content_html: String,
    /// 纯文本正文（已合并空白），index.json 的 content 字段用
    plain: String,
    /// JSON Feed date_published/date_modified 用的 RFC3339（UTC 记 Z）
    date_rfc3339: String,
    image: Option<String>,
    /// 排序/跟 about、guestbook 混排用；不需要序列化给模板
    #[serde(skip)]
    date: DateTime<FixedOffset>,
    /// front matter `sitemap.disable`：sitemap.xml 构建时过滤用，不需要序列化给模板
    #[serde(skip)]
    sitemap_disable: bool,
}

/// 一个 taxonomy term（单个分类/标签）聚合出的页面数据。文章本体只存
/// LangData.cards 里的下标，避免每个 term 抱一份含全文 HTML 的卡片副本。
#[derive(Serialize, Clone)]
struct TermAgg {
    raw: String,
    title: String,
    slug: String,
    rel: String,
    /// 模板里的文章计数（term 云、过滤器角标）
    count: usize,
    /// LangData.cards 的下标（新→旧）
    #[serde(skip)]
    posts: Vec<usize>,
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
            count: 0,
            posts: Vec::new(),
        });
        terms.last_mut().unwrap()
    }
}

/// term 云的排序：按数量倒序，同数量按标题字母序（不区分大小写）
fn sort_terms_by_count(terms: &mut [TermAgg]) {
    terms.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

/// 按 TermAgg 里存的下标取出这个 term 的文章卡片（新→旧）
fn term_cards<'a>(cards: &'a [PostCard], term: &TermAgg) -> Vec<&'a PostCard> {
    term.posts.iter().map(|&i| &cards[i]).collect()
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

// 单进程单次构建，没有并发需求：用 thread_local 存 minify 开关，
// 免得给所有 write_file / render_to 调用点都加一个参数
thread_local! {
    static MINIFY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // 全站内联 <script> 的 sha256（写盘时顺手收集），构建收尾写进
    // csp-hashes.txt，backend 启动时读它拼 Content-Security-Policy。
    // BTreeSet：去重 + 排序，保证产物字节稳定。
    static CSP_HASHES: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
    // 本次构建已写出的渲染产物路径。同一路径写两次一定是配置错误
    // （重复 slug、alias 撞真实页面、term 撞文章），谁后写谁赢会静默
    // 丢页面，必须当场报错。资源与图片走 assets.rs / fs::copy，文件名
    // 含指纹或按 bundle 目录隔离，不在此列。
    static WRITTEN: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

pub fn build(source: &Path, dest: &Path, minify: bool) -> Result<()> {
    MINIFY.with(|m| m.set(minify));
    CSP_HASHES.with(|h| h.borrow_mut().clear());
    WRITTEN.with(|w| w.borrow_mut().clear());
    let config = SiteConfig::load(source)?;
    let lang_codes: Vec<String> = config.languages.iter().map(|l| l.code.clone()).collect();
    let i18n = Arc::new(I18n::load(source, &lang_codes)?);
    let content = content::load(source, &config.default_lang)?;
    let assets = crate::assets::build(source, dest, minify)?;

    publish_bundle_images(&content, dest)?;

    let env = template_env(source, i18n.clone(), assets.urls.clone());

    let mut lang_data: HashMap<String, LangData> = HashMap::new();
    for lang in &config.languages {
        lang_data.insert(
            lang.code.clone(),
            build_lang_posts(&config, &content, &i18n, &lang.code)?,
        );
    }

    for lang in &config.languages {
        let render = LangRender {
            env: &env,
            config: &config,
            content: &content,
            dest,
            theme_js: &assets.theme_js,
            lang,
            prefix: lang.url_prefix(&config.default_lang),
            site: site_ctx(&config, lang, &content),
            data: &lang_data[&lang.code],
        };
        render.render_all(&lang_data)?;
    }

    write_sitemaps(&config, &content, &lang_data, dest)?;

    write_file(
        dest,
        "robots.txt",
        &format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
            config.base_url
        ),
    )?;

    // 内联脚本哈希清单：backend 用它组装 CSP 的 script-src，散列公开无妨。
    // 放正式产物里而不是旁路文件，保证「部署了哪份站点就用哪份清单」。
    let csp_manifest = CSP_HASHES.with(|h| {
        let hashes = h.borrow();
        let mut out = String::new();
        for hash in hashes.iter() {
            out.push_str(hash);
            out.push('\n');
        }
        out
    });
    write_file(dest, "csp-hashes.txt", &csp_manifest)?;

    // 文案完整性检查：渲染过程中任何一次 t() 打空都算构建失败，
    // 不让「构建成功但线上文案空白」这种静默错误溜出去。
    let missing = i18n.missing_keys();
    if !missing.is_empty() {
        anyhow::bail!("i18n 缺少文案 key：{}", missing.join("、"));
    }

    println!("构建完成 → {}", dest.display());
    Ok(())
}

/// 文章 bundle 的图片资源只按默认语言路径发布一份；
/// 大图顺带生成 srcset 用的缩放变体（并行，见 images.rs）
fn publish_bundle_images(content: &Content, dest: &Path) -> Result<()> {
    let mut resize_jobs = Vec::new();
    for bundle in &content.posts {
        let out_dir = dest.join("posts").join(&bundle.slug);
        std::fs::create_dir_all(&out_dir)?;
        for res in &bundle.resources {
            std::fs::copy(&res.disk_path, out_dir.join(&res.name))?;
            if !res.variants.is_empty() {
                resize_jobs.push(crate::images::ResizeJob {
                    src: res.disk_path.clone(),
                    out_dir: out_dir.clone(),
                    name: res.name.clone(),
                    widths: res.variants.clone(),
                });
            }
        }
    }
    crate::images::generate(&resize_jobs)
}

fn template_env(
    source: &Path,
    i18n: Arc<I18n>,
    asset_urls: HashMap<String, String>,
) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_loader(path_loader(source.join("ssg").join("templates")));
    env.set_keep_trailing_newline(true);
    env.set_undefined_behavior(UndefinedBehavior::Chainable);
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
    env.add_function("asset", move |path: String| -> String {
        asset_urls.get(&path).cloned().unwrap_or_else(|| {
            eprintln!("警告: 找不到资源 {path}");
            format!("/{path}")
        })
    });
    env.add_filter("urlquery", |s: String| content::query_escape(&s));
    env
}

fn site_ctx(config: &SiteConfig, lang: &Language, content: &Content) -> SiteCtx {
    let prefix = lang.url_prefix(&config.default_lang);
    // 每种语言的独立页 URL（sidebar / guestbook 链接用）：front matter 的
    // url 是最终绝对路径本身（英文页已经自带 /en 前缀）
    let special_rel = |key: &str| -> String {
        content
            .pages
            .iter()
            .find(|p| p.key == key && p.lang == lang.code)
            .and_then(|p| p.fm.url.clone())
            .unwrap_or_default()
    };
    let other = config.languages.iter().find(|l| l.code != lang.code);
    SiteCtx {
        title: lang.title.clone(),
        lang: lang.code.clone(),
        locale: lang.locale.clone(),
        description: lang.description.clone(),
        base_url: config.base_url.clone(),
        home_rel: lang.home_rel(&config.default_lang),
        author: config.author.clone(),
        current_year: chrono::Utc::now().year(),
        rss_rel: format!("{prefix}/feed.xml"),
        jsonfeed_rel: format!("{prefix}/feed.json"),
        about_rel: special_rel("about"),
        guestbook_rel: special_rel("guestbook"),
        other_lang_home_rel: other
            .map(|l| l.home_rel(&config.default_lang))
            .unwrap_or_default(),
        other_lang_locale: other.map(|l| l.locale.clone()).unwrap_or_default(),
    }
}

/// 一种语言的全部页面渲染。字段都是借用：一次构建里每种语言建一个。
struct LangRender<'a> {
    env: &'a Environment<'static>,
    config: &'a SiteConfig,
    content: &'a Content,
    dest: &'a Path,
    theme_js: &'a str,
    lang: &'a Language,
    prefix: String,
    site: SiteCtx,
    data: &'a LangData,
}

impl LangRender<'_> {
    fn render_all(&self, lang_data: &HashMap<String, LangData>) -> Result<()> {
        let home = home_ctx(self.config, self.lang, self.data)?;
        self.render_page(
            "home.html",
            &format!("{}index.html", self.site.home_rel.trim_start_matches('/')),
            &home,
            minijinja::context! {},
        )?;
        self.home_feeds(&home)?;
        self.notfound()?;
        self.static_pages()?;
        self.archives()?;
        self.taxonomies(lang_data)?;
        self.posts_section()?;
        self.post_pages()?;
        self.post_aliases()
    }

    /// 渲染一个页面模板：base 上下文（lang/site/page/侧栏数据）+ 模板特有的 extra
    fn render_page(
        &self,
        template: &str,
        out_rel: &str,
        page: &PageCtx,
        extra: Value,
    ) -> Result<()> {
        let base = minijinja::context! {
            lang => self.lang.code.clone(),
            site => &self.site,
            page => page,
            posts => &self.data.post_refs,
            timeline => &self.data.timeline,
            theme_js => self.theme_js,
        };
        render_to(
            self.env,
            template,
            self.dest,
            out_rel,
            minijinja::context! { ..extra, ..base },
        )
    }

    /// 把带语言前缀的根相对路径写到产物目录
    fn write(&self, rel: &str, content: &str) -> Result<()> {
        write_file(self.dest, rel.trim_start_matches('/'), content)
    }

    /// 写一份 RSS：channel 元数据取自页面上下文（self_link 用 page.rss_rel）
    fn write_feed(
        &self,
        out_rel: &str,
        page: &PageCtx,
        last_build_date: Option<String>,
        items: &[RssItem],
    ) -> Result<()> {
        let self_rel = page.rss_rel.clone().unwrap_or_default();
        let rss = render_rss(
            &RssChannel {
                title: channel_title(&page.title, &self.lang.title),
                link: page.permalink.clone(),
                description: &page.description,
                site_title: &self.lang.title,
                locale: &self.lang.locale,
                author: &self.config.author,
                last_build_date,
                self_link: format!("{}{self_rel}", self.config.base_url),
                base_url: &self.config.base_url,
            },
            items,
        );
        self.write(out_rel, &rss)
    }

    /// 首页的 RSS / JSON Feed / index.json / guestbook-posts.json
    fn home_feeds(&self, home: &PageCtx) -> Result<()> {
        let config = self.config;
        let lang = self.lang;
        let card_refs: Vec<&PostCard> = self.data.cards.iter().collect();

        // 首页 RSS 是全站内容（文章 + about/guestbook 独立页）按日期混排，
        // 不像 feed.json/index.json 那样只看 posts
        let mut rss_items: Vec<OwnedRssItem> = self
            .data
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
        for raw_page in self
            .content
            .pages
            .iter()
            .filter(|p| p.lang == lang.code && p.kind == PageKind::Page)
        {
            let rendered = markdown::render(&raw_page.body, &[], "");
            let url = raw_page.fm.url.clone().unwrap_or_default();
            rss_items.push(OwnedRssItem {
                title: raw_page.fm.title.clone(),
                link: format!("{}{url}", config.base_url),
                pub_date: gotime::format(&raw_page.date, gotime::RFC1123Z),
                date: raw_page.date,
                categories: Vec::new(),
                content_html: rendered.html,
            });
        }
        rss_items.sort_by_key(|b| std::cmp::Reverse(b.date));
        let last_build_date = rss_items.first().map(|c| c.pub_date.clone());
        let items: Vec<RssItem> = rss_items
            .iter()
            .map(|c| RssItem {
                title: &c.title,
                link: &c.link,
                pub_date: &c.pub_date,
                categories: &c.categories,
                content_html: Some(&c.content_html),
            })
            .collect();
        self.write_feed(
            &format!("{}/feed.xml", self.prefix),
            home,
            last_build_date,
            &items,
        )?;

        self.write(
            &format!("{}/feed.json", self.prefix),
            &feed_json(
                &lang.title,
                &home.permalink,
                &format!("{}{}", config.base_url, self.site.jsonfeed_rel),
                &lang.description,
                &lang.locale,
                &config.author,
                &card_refs,
                &config.base_url,
            ),
        )?;

        self.write(
            &format!("{}/index.json", self.prefix),
            &index_json(&card_refs, lang.has_cjk),
        )?;

        let guestbook_items: Vec<GuestbookItem> = self
            .content
            .posts
            .iter()
            .map(|bundle| {
                let version = bundle
                    .versions
                    .iter()
                    .find(|v| v.lang == lang.code)
                    .or_else(|| bundle.versions.first())
                    .expect("bundle 至少有一个语言版本");
                // 恒用默认语言形态的规范路径：backend 校验 ref_url 只认
                // /posts/ 开头，且留言列表全站共享，引用链接统一指向一处。
                let rel = format!("/posts/{}/", content::encode_path(&bundle.slug));
                GuestbookItem {
                    title: version.fm.title.clone(),
                    rel,
                    date_iso: gotime::format(&version.date, gotime::DATE_ONLY),
                }
            })
            .collect();
        self.write(
            &format!("{}/guestbook-posts.json", self.prefix),
            &guestbook_posts_json(&guestbook_items),
        )
    }

    fn notfound(&self) -> Result<()> {
        let ctx = notfound_ctx(self.config, self.lang)?;
        self.render_page(
            "notfound.html",
            format!("{}/404.html", self.prefix).trim_start_matches('/'),
            &ctx,
            minijinja::context! {},
        )
    }

    /// 独立页面：about / guestbook（跳过 _index 之类的 Section 页，
    /// 它们只是 `build.render: never` 的占位配置页，没有真实 url）
    fn static_pages(&self) -> Result<()> {
        for raw_page in self
            .content
            .pages
            .iter()
            .filter(|p| p.lang == self.lang.code && p.kind == PageKind::Page)
        {
            let rendered = markdown::render(&raw_page.body, &[], "");
            let word_count = if self.lang.has_cjk {
                content::word_count_cjk(&rendered.plain)
            } else {
                content::word_count_latin(&rendered.plain)
            };
            let ctx = static_page_ctx(
                self.config,
                self.lang,
                self.content,
                raw_page,
                rendered.html,
                word_count,
            )?;
            let template = match raw_page.fm.layout.as_deref() {
                Some("guestbook") => "guestbook.html",
                _ => "about.html",
            };
            let out_rel = format!("{}index.html", ctx.rel_permalink.trim_start_matches('/'));
            self.render_page(
                template,
                &out_rel,
                &ctx,
                minijinja::context! {
                    post_count_fmt => format_number(self.data.post_refs.len()),
                    total_words_fmt => format_number(self.data.total_words),
                },
            )?;
        }
        Ok(())
    }

    fn archives(&self) -> Result<()> {
        let Some(raw_page) = self
            .content
            .archives
            .iter()
            .find(|p| p.lang == self.lang.code)
        else {
            return Ok(());
        };
        let rendered = markdown::render(&raw_page.body, &[], "");
        let ctx = archives_ctx(
            self.config,
            self.lang,
            self.content,
            raw_page,
            rendered.html,
        )?;
        let out_rel = format!("{}index.html", ctx.rel_permalink.trim_start_matches('/'));

        // 时间线视图：按年分组（cards 已按日期倒序）
        let mut archive_years: Vec<YearCards> = Vec::new();
        for card in &self.data.cards {
            match archive_years.last_mut() {
                Some(group) if group.year == card.year => group.cards.push(card),
                _ => archive_years.push(YearCards {
                    year: card.year.clone(),
                    cards: vec![card],
                }),
            }
        }
        let mut categories_by_count = self.data.categories.clone();
        sort_terms_by_count(&mut categories_by_count);
        let mut tags_by_count = self.data.tags.clone();
        sort_terms_by_count(&mut tags_by_count);

        self.render_page(
            "archives.html",
            &out_rel,
            &ctx,
            minijinja::context! {
                archive_cards => &self.data.cards,
                archive_years => &archive_years,
                categories_by_count => &categories_by_count,
                tags_by_count => &tags_by_count,
                total_posts => self.data.cards.len(),
                all_active => true,
                active_category_rel => Option::<String>::None,
            },
        )
    }

    /// 分类/标签：列表页 + 各 term 页（term 页带自己的 feed.xml）
    fn taxonomies(&self, lang_data: &HashMap<String, LangData>) -> Result<()> {
        let config = self.config;
        for (taxonomy, singular, list_title, terms) in [
            (
                "categories",
                "category",
                "Categories",
                &self.data.categories,
            ),
            ("tags", "tag", "Tags", &self.data.tags),
        ] {
            let list_rel = format!("{}/{taxonomy}/", self.prefix);
            let mut sorted_terms = terms.clone();
            sort_terms_by_count(&mut sorted_terms);

            let list_ctx = terms_list_ctx(
                config,
                self.lang,
                taxonomy,
                singular,
                list_title,
                self.data.newest_date,
            )?;
            self.render_page(
                "taxonomy-list.html",
                &format!("{}index.html", list_rel.trim_start_matches('/')),
                &list_ctx,
                minijinja::context! { terms => &sorted_terms },
            )?;

            for term in &sorted_terms {
                // 按 config.languages 的固定顺序（zh 在前）列出这个 term
                // 在每种语言下的对应页——不管当前渲染的是哪个语言，顺序都一样
                let all_translations: Vec<TranslationRef> = config
                    .languages
                    .iter()
                    .filter_map(|l| {
                        let r = if l.code == self.lang.code {
                            term.rel.clone()
                        } else {
                            let other_terms = if taxonomy == "categories" {
                                &lang_data[&l.code].categories
                            } else {
                                &lang_data[&l.code].tags
                            };
                            let other_term = other_terms.iter().find(|t| t.slug == term.slug)?;
                            format!(
                                "{}/{taxonomy}/{}/",
                                l.url_prefix(&config.default_lang),
                                content::encode_path(&other_term.slug)
                            )
                        };
                        Some(TranslationRef {
                            lang: l.code.clone(),
                            locale: l.locale.clone(),
                            permalink: format!("{}{r}", config.base_url),
                            rel: r,
                        })
                    })
                    .collect();

                let cards = term_cards(&self.data.cards, term);
                let newest = cards.iter().map(|c| c.date).max();
                let ctx = term_ctx(config, self.lang, singular, term, newest, all_translations)?;
                // 文件系统路径要用没编码过的原始 slug（磁盘目录名就是真实的
                // UTF-8 字符），不能用 term.rel——那个是给 href/canonical 用的
                // 百分号编码形式，两者只在纯 ASCII slug 时恰好相同
                let disk_dir = format!("{}/{taxonomy}/{}/", self.prefix, term.slug)
                    .trim_start_matches('/')
                    .to_string();
                self.render_page(
                    "taxonomy-term.html",
                    &format!("{disk_dir}index.html"),
                    &ctx,
                    minijinja::context! {
                        cards => &cards,
                        is_category => taxonomy == "categories",
                    },
                )?;

                let last_build_date = newest.map(|d| gotime::format(&d, gotime::RFC1123Z));
                self.write_feed(
                    &format!("{disk_dir}feed.xml"),
                    &ctx,
                    last_build_date,
                    &card_rss_items(&cards),
                )?;
            }
        }
        Ok(())
    }

    /// posts 这个 section 的隐式列表页（只有 build.render 没被关掉的语言才
    /// 渲染——本站只有英文）；7 篇 > pagerSize 6，真的会分页
    fn posts_section(&self) -> Result<()> {
        let Some(raw_page) =
            self.content.posts_section.iter().find(|p| {
                p.lang == self.lang.code && p.fm.build.render.as_deref() != Some("never")
            })
        else {
            return Ok(());
        };
        const PAGE_SIZE: usize = 6;
        let rel = format!("{}/posts/", self.prefix);
        let total_pages = self.data.cards.len().div_ceil(PAGE_SIZE).max(1);
        let page_url = |n: usize| -> String {
            if n <= 1 {
                rel.clone()
            } else {
                format!("{rel}p/{n}/")
            }
        };
        let ctx = posts_section_ctx(
            self.config,
            self.lang,
            raw_page,
            rel.clone(),
            self.data.newest_date,
        )?;
        let pager_pages: Vec<PagerPage> = (1..=total_pages)
            .map(|n| PagerPage {
                number: n,
                url: page_url(n),
            })
            .collect();

        for (idx, chunk) in self.data.cards.chunks(PAGE_SIZE).enumerate() {
            let n = idx + 1;
            let out_rel = format!("{}index.html", page_url(n).trim_start_matches('/'));
            self.render_page(
                "posts-list.html",
                &out_rel,
                &ctx,
                minijinja::context! {
                    cards => chunk,
                    pager_current => n,
                    pager_total => total_pages,
                    pager_first_url => page_url(1),
                    pager_prev_url => if n > 1 { Some(page_url(n - 1)) } else { None },
                    pager_next_url => if n < total_pages { Some(page_url(n + 1)) } else { None },
                    pager_last_url => page_url(total_pages),
                    pager_pages => &pager_pages,
                },
            )?;
        }

        // 这个 section 自己的 feed.xml：全部文章，不分页
        let last_build_date = self
            .data
            .newest_date
            .map(|d| gotime::format(&d, gotime::RFC1123Z));
        let card_refs: Vec<&PostCard> = self.data.cards.iter().collect();
        self.write_feed(
            &format!("{rel}feed.xml"),
            &ctx,
            last_build_date,
            &card_rss_items(&card_refs),
        )
    }

    fn post_pages(&self) -> Result<()> {
        for page in &self.data.posts {
            let out_rel = format!("{}index.html", page.rel_permalink.trim_start_matches('/'));
            self.render_page("single.html", &out_rel, page, minijinja::context! {})?;
        }
        Ok(())
    }

    /// front matter aliases 声明的旧地址 → meta-refresh 跳转桩
    fn post_aliases(&self) -> Result<()> {
        for bundle in &self.content.posts {
            let Some(version) = bundle.versions.iter().find(|v| v.lang == self.lang.code) else {
                continue;
            };
            let permalink = format!(
                "{}{}/posts/{}/",
                self.config.base_url,
                self.prefix,
                content::encode_path(&bundle.slug)
            );
            for alias in &version.fm.aliases {
                let out = format!("{}{alias}index.html", self.prefix);
                self.write(&out, &alias_html(&self.lang.locale, &permalink))?;
            }
        }
        Ok(())
    }
}

/// sitemap.xml：根 sitemapindex 汇总各语言，各语言一份 urlset
fn write_sitemaps(
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
    items.sort_by_key(|b| std::cmp::Reverse(b.1.date));

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
            date_iso: gotime::format(&page.date, gotime::DATE_ONLY),
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
            word_count.div_ceil(501)
        } else {
            word_count.div_ceil(213)
        };
        let h2_count = rendered.toc.iter().filter(|t| t.level == 2).count();
        let has_toc = h2_count > 1;
        let permalink = format!("{}{rel}", config.base_url);
        // 没写 description 时兜底用正文摘要，而不是留空——home/archives 已经有
        // 站点级兜底（lang.description），单篇文章没有，不然 meta
        // description/og/twitter 描述整块留空
        let description =
            page.fm.description.clone().unwrap_or_else(|| {
                plain_summary(&rendered.plain, config.summary_length, lang.has_cjk)
            });
        let date_rfc = gotime::format(&page.date, gotime::RFC3339);

        let all_translations = translations_of(config, bundle);
        let tags: Vec<TermRef> = page.fm.tags.iter().map(|t| term_ref("tags", t)).collect();
        let categories: Vec<TermRef> = page
            .fm
            .categories
            .iter()
            .map(|c| term_ref("categories", c))
            .collect();
        // article:tag / jsonld keywords 用 term 的自动 title（每词首字母大写），
        // 不是 front matter 里的原始大小写
        let og_tags: Vec<String> = page
            .fm
            .tags
            .iter()
            .map(|t| content::hugo_title_case(t))
            .collect();

        // 正文里出现的第一张图（不管 front matter 有没有声明），给 og:image
        // 和 jsonld 的兜底用
        let cover_abs = rendered
            .first_image_src
            .as_ref()
            .map(|src| format!("{}{src}", config.base_url));
        let fallback_img = config.default_image();

        // og:image / twitter:image / itemprop="image"：优先 front matter
        // images（按 bundle 资源解析出正确的 /posts/slug/ 前缀），没声明就
        // 退到正文首图——Galgame 感想篇篇有截图，分享卡片用正文首图远比
        // 站点通用图贴切——都没有才落到站点默认图。
        let og_image = page
            .fm
            .images
            .first()
            .map(|img| resolve_bundle_image(config, bundle, &bundle_rel, img))
            .or_else(|| cover_abs.clone())
            .or_else(|| fallback_img.clone());

        // jsonld 的 image：front matter images 直接绝对化、不经过 bundle
        // 解析；没声明才退到正文首图，最后是站点默认图
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
        let mut meta = InternalMeta::new(
            lang,
            config,
            &permalink,
            &page.fm.title,
            &description,
            &date_rfc,
        );
        meta.og_type = "article";
        meta.section = Some("posts");
        meta.tags = &og_tags;
        meta.image = og_image.clone().unwrap_or_default();
        meta.word_count = Some(word_count);
        meta.jsonld = Some(&jsonld);
        let internal_meta = internal_meta_block(&meta);

        // 归档卡片封面永远是正文里第一张图，跟 og:image 的解析逻辑（会优先
        // 用 front matter images）是两条不同的路径，故意不共用。
        // 卡片封面常态显示宽度只有几百 px：src 取 768w 变体兜底，
        // 别让一张卡片缩略图拖整张 2560 原图。但归档网格是 auto-fit，
        // 筛选后只剩一张卡时会拉到整个内容栏宽（最宽 1060px CSS 像素），
        // 所以 srcset 带上全部档位，靠模板里的 sizes=auto 按真实布局宽度换档。
        let cover_resource = rendered.first_image_src.as_deref().and_then(|src| {
            bundle
                .resources
                .iter()
                .find(|r| format!("{bundle_rel}{}", r.name) == src)
        });
        let cover_url = rendered.first_image_src.clone().map(|src| {
            cover_resource
                .filter(|r| r.variants.contains(&768))
                .map(|r| format!("{bundle_rel}{}", crate::images::variant_name(&r.name, 768)))
                .unwrap_or(src)
        });
        let cover_srcset = cover_resource.filter(|r| !r.variants.is_empty()).map(|r| {
            let mut candidates: Vec<String> = r
                .variants
                .iter()
                .map(|w| {
                    format!(
                        "{bundle_rel}{} {w}w",
                        crate::images::variant_name(&r.name, *w)
                    )
                })
                .collect();
            candidates.push(format!("{bundle_rel}{} {}w", r.name, r.width));
            candidates.join(", ")
        });
        let card = PostCard {
            title: page.fm.title.clone(),
            rel: rel.clone(),
            permalink: permalink.clone(),
            date_iso: post_refs[idx].date_iso.clone(),
            date_display: post_refs[idx].date_display.clone(),
            date_mmdd: post_refs[idx].date_mmdd.clone(),
            year: post_refs[idx].year.clone(),
            pub_date: gotime::format(&page.date, gotime::RFC1123Z),
            word_count,
            image_count: rendered.html.matches("<img").count(),
            summary: description.clone(),
            categories_attr: json_attr(&page.fm.categories),
            tags_attr: json_attr(&page.fm.tags),
            categories_raw: page.fm.categories.clone(),
            tags_raw: page.fm.tags.clone(),
            tags_title: og_tags.clone(),
            cover_url,
            cover_srcset,
            content_html: rendered.html.clone(),
            plain: collapse_ws(&html_unescape_typographic(&rendered.plain)),
            date_rfc3339: gotime::format(&page.date, gotime::RFC3339Z),
            // JSON Feed 的 image：只认 front matter images，没有站点默认图兜底
            // （跟 og:image 不一样），绝对化但不经过 bundle 解析
            image: page
                .fm
                .images
                .first()
                .map(|img| format!("{}/{img}", config.base_url)),
            date: page.date,
            sitemap_disable: page.fm.sitemap.disable,
        };
        for cat in &page.fm.categories {
            let tref = term_ref("categories", cat);
            term_agg(&mut cat_terms, cat, &tref).posts.push(idx);
        }
        for tag in &page.fm.tags {
            let tref = term_ref("tags", tag);
            term_agg(&mut tag_terms, tag, &tref).posts.push(idx);
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
            counter_path: bundle_rel.clone(),
            date_iso: gotime::format(&page.date, gotime::DATE_ONLY),
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
    for term in cat_terms.iter_mut().chain(tag_terms.iter_mut()) {
        term.count = term.posts.len();
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

fn home_ctx(config: &SiteConfig, lang: &Language, data: &LangData) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = lang.home_rel(&config.default_lang);
    let permalink = format!("{}{rel}", config.base_url);
    let date_rfc = data
        .newest_date
        .map(|d| gotime::format(&d, gotime::RFC3339))
        .unwrap_or_default();

    // 首页的语言切换目标：另一语言的首页
    let all_translations = config
        .languages
        .iter()
        .map(|l| {
            let r = l.home_rel(&config.default_lang);
            TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            }
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        &lang.title,
        &lang.description,
        &date_rfc,
    ));

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

/// 独立页面（about/guestbook）：og:type=article、section=pages
fn static_page_ctx(
    config: &SiteConfig,
    lang: &Language,
    content: &Content,
    page: &RawPage,
    content_html: String,
    word_count: usize,
) -> Result<PageCtx> {
    let url = page.fm.url.clone().unwrap_or_default();
    let permalink = format!("{}{url}", config.base_url);
    let description = page.fm.description.clone().unwrap_or_default();
    let date_rfc = gotime::format(&page.date, gotime::RFC3339);

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

    let mut meta = InternalMeta::new(
        lang,
        config,
        &permalink,
        &page.fm.title,
        &description,
        &date_rfc,
    );
    meta.og_type = "article";
    meta.section = Some("pages");
    meta.word_count = Some(word_count);
    let internal_meta = internal_meta_block(&meta);

    let (canonical, redirect_block) = redirect_fields(config, page.fm.redirect_to.as_deref())
        .unwrap_or_else(|| (permalink.clone(), String::new()));

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

/// archives 页：og:type=website（section 页而非 article）；URL 是算出来的
/// （content/archives/_index.md 没有自己的 url 字段，不像 about/guestbook）
fn archives_ctx(
    config: &SiteConfig,
    lang: &Language,
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
        gotime::format(&page.date, gotime::RFC3339)
    };

    let all_translations = config
        .languages
        .iter()
        .filter_map(|l| {
            content.archives.iter().find(|p| p.lang == l.code)?;
            let r = format!("{}/archives/", l.url_prefix(&config.default_lang));
            Some(TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            })
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        &page.fm.title,
        &description,
        &date_rfc,
    ));

    let (canonical, redirect_block) = redirect_fields(config, page.fm.redirect_to.as_deref())
        .unwrap_or_else(|| (permalink.clone(), String::new()));

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
        permalink,
        content_html,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

/// 单个 term 页（如 /tags/atri/）：没有专属内容，标题/描述都是自动生成的
/// （标题 = term 的自动 title，描述退到站点描述），日期取这个 term 下
/// 最新一篇文章的日期
fn term_ctx(
    config: &SiteConfig,
    lang: &Language,
    taxonomy_singular: &str,
    term: &TermAgg,
    newest: Option<DateTime<FixedOffset>>,
    all_translations: Vec<TranslationRef>,
) -> Result<PageCtx> {
    let permalink = format!("{}{}", config.base_url, term.rel);
    let description = lang.description.clone();
    let date_rfc = newest
        .map(|d| gotime::format(&d, gotime::RFC3339))
        .unwrap_or_default();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        &term.title,
        &description,
        &date_rfc,
    ));

    Ok(PageCtx {
        kind: "term".into(),
        section: taxonomy_singular.into(),
        title: term.title.clone(),
        description_meta: collapse_ws(&description),
        description,
        rel_permalink: term.rel.clone(),
        canonical: permalink.clone(),
        rss_rel: Some(format!("{}feed.xml", term.rel)),
        permalink,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

/// taxonomy 列表页（/categories/、/tags/）：标题固定用英文复数，
/// 两种语言都一样
fn terms_list_ctx(
    config: &SiteConfig,
    lang: &Language,
    taxonomy: &str,
    taxonomy_singular: &str,
    title: &str,
    newest_date: Option<DateTime<FixedOffset>>,
) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = format!("{prefix}/{taxonomy}/");
    let permalink = format!("{}{rel}", config.base_url);
    let description = lang.description.clone();
    let date_rfc = newest_date
        .map(|d| gotime::format(&d, gotime::RFC3339))
        .unwrap_or_default();

    // 每种语言各自独立算 rel（不能拿当前语言已经拼好前缀的 rel 去给
    // 别的语言复用），按 config.languages 的固定顺序排列
    let all_translations = config
        .languages
        .iter()
        .map(|l| {
            let r = format!("{}/{taxonomy}/", l.url_prefix(&config.default_lang));
            TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            }
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        title,
        &description,
        &date_rfc,
    ));

    Ok(PageCtx {
        kind: "terms".into(),
        section: taxonomy_singular.into(),
        title: title.into(),
        description_meta: collapse_ws(&description),
        description,
        canonical: permalink.clone(),
        rel_permalink: rel,
        permalink,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

/// posts 这个 section 的隐式列表页（本站只有英文端会真的渲染：中文端
/// content/posts/_index.md 有 build.render: never 关掉了）；带
/// redirectTo（canonical 到 /en/archives/，同时注入 meta-refresh 跳转）
fn posts_section_ctx(
    config: &SiteConfig,
    lang: &Language,
    page: &RawPage,
    rel: String,
    newest_date: Option<DateTime<FixedOffset>>,
) -> Result<PageCtx> {
    let permalink = format!("{}{rel}", config.base_url);
    let description = page.fm.description.clone().unwrap_or_default();
    let date_rfc = newest_date
        .map(|d| gotime::format(&d, gotime::RFC3339))
        .unwrap_or_default();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        &page.fm.title,
        &description,
        &date_rfc,
    ));

    let (canonical, redirect_block) = redirect_fields(config, page.fm.redirect_to.as_deref())
        .unwrap_or_else(|| (permalink.clone(), String::new()));

    // 中文端 build.render: never 关掉了，这个 section 只有自己这一个语言
    // 版本，all_translations 里只有它自己
    let all_translations = vec![TranslationRef {
        lang: lang.code.clone(),
        locale: lang.locale.clone(),
        permalink: permalink.clone(),
        rel: rel.clone(),
    }];

    Ok(PageCtx {
        kind: "posts-section".into(),
        title: page.fm.title.clone(),
        description_meta: collapse_ws(&description),
        description,
        rel_permalink: rel.clone(),
        canonical,
        redirect_block,
        rss_rel: Some(format!("{rel}feed.xml")),
        permalink,
        all_translations,
        internal_meta,
        ..Default::default()
    })
}

fn notfound_ctx(config: &SiteConfig, lang: &Language) -> Result<PageCtx> {
    let prefix = lang.url_prefix(&config.default_lang);
    let rel = format!("{prefix}/404.html");
    let permalink = format!("{}{rel}", config.base_url);

    let all_translations = config
        .languages
        .iter()
        .map(|l| {
            let r = format!("{}/404.html", l.url_prefix(&config.default_lang));
            TranslationRef {
                lang: l.code.clone(),
                locale: l.locale.clone(),
                permalink: format!("{}{r}", config.base_url),
                rel: r,
            }
        })
        .collect();

    let internal_meta = internal_meta_block(&InternalMeta::new(
        lang,
        config,
        &permalink,
        "404 Page not found",
        &lang.description,
        "",
    ));

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
    locale: String,
    og_type: &'a str,
    section: Option<&'a str>,
    date_published: &'a str,
    date_modified: &'a str,
    tags: &'a [String],
    image: String,
    word_count: Option<usize>,
    jsonld: Option<&'a str>,
}

impl<'a> InternalMeta<'a> {
    /// og:type=website、无 tags/jsonld 的基础形态；文章/独立页在此之上覆写
    /// og_type / section / tags / image / word_count / jsonld
    fn new(
        lang: &'a Language,
        config: &SiteConfig,
        permalink: &'a str,
        title: &'a str,
        description: &'a str,
        date_rfc: &'a str,
    ) -> Self {
        InternalMeta {
            permalink,
            site_title: &lang.title,
            title,
            description,
            locale: lang.locale.replace('-', "_"),
            og_type: "website",
            section: None,
            date_published: date_rfc,
            date_modified: date_rfc,
            tags: &[],
            image: config.default_image().unwrap_or_default(),
            word_count: None,
            jsonld: None,
        }
    }
}

/// opengraph / twitter cards / schema.org microdata / JSON-LD 的整块 head 输出
fn internal_meta_block(m: &InternalMeta) -> String {
    let esc = escape_attr;
    let mut lines = vec![
        format!(
            "<meta property=\"og:url\" content=\"{}\">",
            esc(m.permalink)
        ),
        format!(
            "<meta property=\"og:site_name\" content=\"{}\">",
            esc(m.site_title)
        ),
        format!("<meta property=\"og:title\" content=\"{}\">", esc(m.title)),
        format!(
            "<meta property=\"og:description\" content=\"{}\">",
            esc(m.description)
        ),
        format!(
            "<meta property=\"og:locale\" content=\"{}\">",
            esc(&m.locale)
        ),
        format!("<meta property=\"og:type\" content=\"{}\">", m.og_type),
    ];
    if m.og_type == "article" {
        if let Some(section) = m.section {
            lines.push(format!(
                "<meta property=\"article:section\" content=\"{}\">",
                esc(section)
            ));
        }
        lines.push(format!(
            "<meta property=\"article:published_time\" content=\"{}\">",
            m.date_published
        ));
        lines.push(format!(
            "<meta property=\"article:modified_time\" content=\"{}\">",
            m.date_modified
        ));
        for tag in m.tags {
            lines.push(format!(
                "<meta property=\"article:tag\" content=\"{}\">",
                esc(tag)
            ));
        }
    }
    lines.push(format!(
        "<meta property=\"og:image\" content=\"{}\">",
        esc(&m.image)
    ));

    lines.push("<meta name=\"twitter:card\" content=\"summary_large_image\">".to_string());
    lines.push(format!(
        "<meta name=\"twitter:image\" content=\"{}\">",
        esc(&m.image)
    ));
    lines.push(format!(
        "<meta name=\"twitter:title\" content=\"{}\">",
        esc(m.title)
    ));
    lines.push(format!(
        "<meta name=\"twitter:description\" content=\"{}\">",
        esc(m.description)
    ));

    lines.push(format!(
        "<meta itemprop=\"name\" content=\"{}\">",
        esc(m.title)
    ));
    lines.push(format!(
        "<meta itemprop=\"description\" content=\"{}\">",
        esc(m.description)
    ));
    if !m.date_published.is_empty() {
        lines.push(format!(
            "<meta itemprop=\"datePublished\" content=\"{}\">",
            m.date_published
        ));
        lines.push(format!(
            "<meta itemprop=\"dateModified\" content=\"{}\">",
            m.date_modified
        ));
    }
    if let Some(wc) = m.word_count {
        if wc > 0 {
            lines.push(format!("<meta itemprop=\"wordCount\" content=\"{wc}\">"));
        }
    }
    lines.push(format!(
        "<meta itemprop=\"image\" content=\"{}\">",
        esc(&m.image)
    ));
    if !m.tags.is_empty() {
        lines.push(format!(
            "<meta itemprop=\"keywords\" content=\"{}\">",
            esc(&m.tags.join(","))
        ));
    }
    if let Some(jsonld) = m.jsonld {
        lines.push(format!(
            "<script type=\"application/ld+json\">{jsonld}</script>"
        ));
    }
    // baseof 里的插入点是 2 格缩进，行间用同样的缩进对齐
    lines.join("\n  ")
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
    // serde_json 开了 preserve_order：手动按字母序 insert，保证产物字节稳定
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

/// 别名/语言桩用的 meta-refresh 跳转页
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
    let duplicate = WRITTEN.with(|w| !w.borrow_mut().insert(rel.to_string()));
    if duplicate {
        anyhow::bail!(
            "输出路径冲突：{rel} 被写入两次（检查重复 slug / aliases / term 是否撞路径）"
        );
    }
    let path = dest.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let should_minify = rel.ends_with(".html") && MINIFY.with(|m| m.get());
    let bytes: std::borrow::Cow<[u8]> = if should_minify {
        std::borrow::Cow::Owned(minify_html_doc(content))
    } else {
        std::borrow::Cow::Borrowed(content.as_bytes())
    };
    // CSP 哈希对最终落盘字节算（minify 之后），跟浏览器看到的完全一致
    if rel.ends_with(".html") {
        CSP_HASHES.with(|h| collect_inline_script_hashes(&bytes, &mut h.borrow_mut()));
    }
    std::fs::write(&path, bytes).with_context(|| format!("写入 {} 失败", path.display()))
}

/// 收集 HTML 里可执行内联 <script> 的 sha256（base64），给 CSP script-src
/// 用。跳过带 src 的外链脚本；也跳过 application/ld+json——数据块不参与
/// 执行，不受 script-src 约束，而且每页内容都不同，收进去只会把头撑爆。
fn collect_inline_script_hashes(html: &[u8], out: &mut std::collections::BTreeSet<String>) {
    use sha2::{Digest, Sha256};
    let mut i = 0;
    while let Some(pos) = find_bytes(&html[i..], b"<script").map(|p| i + p) {
        // 排除 <scriptxxx> 这类假匹配：标签名后必须是空白或 '>'
        let after = html.get(pos + b"<script".len()).copied();
        let Some(tag_close) = find_bytes(&html[pos..], b">").map(|p| pos + p) else {
            return;
        };
        let content_start = tag_close + 1;
        let Some(content_end) =
            find_bytes(&html[content_start..], b"</script").map(|p| content_start + p)
        else {
            return;
        };
        i = content_end + b"</script".len();
        if !matches!(after, Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n')) {
            continue;
        }
        let tag = &html[pos..content_start];
        if find_bytes(tag, b" src").is_some() || find_bytes(tag, b"ld+json").is_some() {
            continue;
        }
        let body = &html[content_start..content_end];
        if body.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        use base64::Engine;
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));
        out.insert(format!("sha256-{hash}"));
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `--minify`：压缩 HTML，顺带压缩 `<style>`/`style=` 里的 CSS。
/// JS 故意不让 minify-html 碰：它内部用 minify-js 压 `<script>`，这个
/// crate 不仅会对合法写法内部断言 panic，还会产出不 panic 但真的跑错的
/// 代码（把函数声明提到它引用的块作用域变量外面，运行时 ReferenceError），
/// 关于页运行时长脚本就是这么线上炸的，见 assets.rs 的说明。
fn minify_html_doc(html: &str) -> Vec<u8> {
    let mut cfg = minify_html::Cfg::new();
    cfg.minify_css = true;
    cfg.minify_js = false;
    // 就算只压 HTML/CSS，minify-html 内部仍然可能触发意外 panic；
    // catch_unwind 兜底，panic 就整页退回不压缩，不让一次异常拖垮整个构建
    let bytes = html.as_bytes();
    std::panic::catch_unwind(|| minify_html::minify(bytes, &cfg)).unwrap_or_else(|_| bytes.to_vec())
}

/// 千位加英文逗号分隔（关于页统计数字用）
fn format_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 把纯文本里残留的 HTML 实体解码回真实字符（typographer 输出的弯引号/
/// 破折号/省略号，以及被原样转义出来的 `&quot;`/`&#34;` 等）——
/// index.json 与兜底摘要用的是解码后的纯文本形态
fn html_unescape_typographic(s: &str) -> String {
    s.replace("&ldquo;", "\u{201c}")
        .replace("&rdquo;", "\u{201d}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&rsquo;", "\u{2019}")
        .replace("&hellip;", "\u{2026}")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 把 front matter `images` 里的文件名解析成完整 URL：命中 bundle 内资源就
/// 带上 /posts/slug/ 前缀，否则当站点根路径处理
fn resolve_bundle_image(
    config: &SiteConfig,
    bundle: &PostBundle,
    bundle_rel: &str,
    img: &str,
) -> String {
    if bundle.resources.iter().any(|r| r.name == *img) {
        format!("{}{bundle_rel}{img}", config.base_url)
    } else {
        format!("{}/{img}", config.base_url)
    }
}

/// redirectTo 存在时算出 (canonical 绝对地址, meta-refresh+JS 跳转整块)
fn redirect_fields(config: &SiteConfig, redirect_to: Option<&str>) -> Option<(String, String)> {
    let target = redirect_to?;
    // canonical 和 meta-refresh 都把 target 插进 HTML 属性，必须一样转义；
    // target 里带 "/& 之类字符会撑破 <link> 标签
    let abs = format!("{}{}", config.base_url, escape_attr(target));
    let js = serde_json::to_string(target).unwrap_or_default();
    let block = format!(
        "\n    <meta http-equiv=\"refresh\" content=\"0; url={}\">\n    <script>window.location.replace({js});</script>\n  ",
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

/// `data-categories`/`data-tags` 属性：JSON 数组落进一个 HTML 属性值，
/// 要走属性转义，不能像 `<script>` 里那样原样插入
fn json_attr(items: &[String]) -> String {
    escape_attr(&serde_json::to_string(items).unwrap_or_default())
}

/// RSS 单个条目
struct RssItem<'a> {
    title: &'a str,
    link: &'a str,
    pub_date: &'a str,
    /// RSS `<category>`：term 的自动 title
    categories: &'a [String],
    /// 完整正文（站内根相对 src/href 会改成绝对地址后再 HTML 转义）
    content_html: Option<&'a str>,
}

/// 首页 RSS 的条目是全站内容（文章 + about/guestbook 独立页），所以单独存
/// 一份带日期、拥有所有权的列表，按日期混排后再借出 RssItem
struct OwnedRssItem {
    title: String,
    link: String,
    pub_date: String,
    date: DateTime<FixedOffset>,
    categories: Vec<String>,
    content_html: String,
}

/// RSS channel 元数据
struct RssChannel<'a> {
    title: String,
    link: String,
    description: &'a str,
    site_title: &'a str,
    locale: &'a str,
    author: &'a str,
    last_build_date: Option<String>,
    self_link: String,
    base_url: &'a str,
}

/// 文章卡片 → RSS 条目
fn card_rss_items<'a>(cards: &[&'a PostCard]) -> Vec<RssItem<'a>> {
    cards
        .iter()
        .map(|&c| RssItem {
            title: &c.title,
            link: &c.permalink,
            pub_date: &c.pub_date,
            categories: &c.tags_title,
            content_html: Some(&c.content_html),
        })
        .collect()
}

fn render_rss(ch: &RssChannel, items: &[RssItem]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    s.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    s.push_str("  <channel>\n");
    s.push_str(&format!("    <title>{}</title>\n", escape_attr(&ch.title)));
    s.push_str(&format!("    <link>{}</link>\n", ch.link));
    s.push_str(&format!(
        "    <description>{}</description>\n",
        escape_attr(ch.description)
    ));
    s.push_str(&format!(
        "    <generator>{}</generator>\n",
        escape_attr(ch.site_title)
    ));
    s.push_str(&format!("    <language>{}</language>\n", ch.locale));
    s.push_str(&format!(
        "    <managingEditor>{}</managingEditor>\n",
        ch.author
    ));
    s.push_str(&format!("    <webMaster>{}</webMaster>\n", ch.author));
    s.push_str(&format!(
        "    <copyright>© {} {}</copyright>\n",
        chrono::Utc::now().year(),
        ch.author
    ));
    if let Some(lbd) = &ch.last_build_date {
        s.push_str(&format!("    <lastBuildDate>{lbd}</lastBuildDate>\n"));
    }
    s.push_str(&format!(
        "    <atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\" />\n",
        ch.self_link
    ));
    for item in items {
        s.push_str("    <item>\n");
        s.push_str(&format!(
            "      <title>{}</title>\n",
            escape_attr(item.title)
        ));
        s.push_str(&format!("      <link>{}</link>\n", item.link));
        s.push_str(&format!("      <pubDate>{}</pubDate>\n", item.pub_date));
        s.push_str(&format!("      <guid>{}</guid>\n", item.link));
        for cat in item.categories {
            s.push_str(&format!(
                "      <category>{}</category>\n",
                escape_attr(cat)
            ));
        }
        let content = item
            .content_html
            .map(|html| {
                html.replace("src=\"/", &format!("src=\"{}/", ch.base_url))
                    .replace("href=\"/", &format!("href=\"{}/", ch.base_url))
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

/// feed channel 标题：页面标题 · 站点标题（页面标题为空或等于站点标题时
/// 只保留站点标题）
fn channel_title(page_title: &str, site_title: &str) -> String {
    if page_title == site_title || page_title.is_empty() {
        site_title.to_string()
    } else {
        format!("{page_title} · {site_title}")
    }
}

/// 文章没写 description 时的兜底摘要：CJK 语言没有空白分词，按字符数截前
/// `max_words` 个；拉丁语系按空白分词截前 `max_words` 个词，避免切碎单词。
/// （site.toml `summaryLength`，默认 70）
fn plain_summary(plain: &str, max_words: usize, has_cjk: bool) -> String {
    let trimmed = plain.trim();
    if has_cjk {
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() <= max_words {
            return trimmed.to_string();
        }
        let excerpt: String = chars[..max_words].iter().collect();
        format!("{excerpt}…")
    } else {
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        if words.len() <= max_words {
            return trimmed.to_string();
        }
        format!("{}…", words[..max_words].join(" "))
    }
}

/// index.json 的 content 截断：CJK 硬切在第 `max` 个字符（连续汉字之间
/// 没有分词，切哪都一样）；空格分词的语言切在词边界——第 `max` 个字符
/// 本身是空白就直接切，切在词中间则回退到前一个空白、丢掉半个单词。
/// 截断后补 " …"。
fn truncate_runes(s: &str, max: usize, has_cjk: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let cut = if has_cjk || chars[max].is_whitespace() {
        max
    } else {
        (0..max)
            .rev()
            .find(|&i| chars[i].is_whitespace())
            .unwrap_or(max)
    };
    let mut out: String = chars[..cut].iter().collect();
    out.push_str(" …");
    out
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

/// 手动控制键顺序的 JSON 对象（serde_json 开了 preserve_order，按插入顺序
/// 输出），保证产物字节稳定
fn ordered_json(pairs: Vec<(&str, serde_json::Value)>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    serde_json::Value::Object(map)
}

/// index.json：全部文章的极简摘要列表（url/title/date/tags/content），
/// 站内搜索的兜底索引
fn index_json(items: &[&PostCard], has_cjk: bool) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|c| {
            ordered_json(vec![
                (
                    "content",
                    serde_json::json!(truncate_runes(&c.plain, 800, has_cjk)),
                ),
                ("date", serde_json::json!(c.date_display)),
                ("tags", serde_json::json!(c.tags_raw)),
                ("title", serde_json::json!(c.title)),
                ("url", serde_json::json!(c.rel)),
            ])
        })
        .collect();
    serde_json::to_string(&ordered_json(vec![("items", serde_json::json!(arr))]))
        .unwrap_or_default()
}

/// guestbook-posts.json：留言板文章选择器用的极简列表
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
    serde_json::to_string(&ordered_json(vec![("items", serde_json::json!(arr))]))
        .unwrap_or_default()
}

struct GuestbookItem {
    title: String,
    rel: String,
    date_iso: String,
}

/// feed.json（JSON Feed 1.1）
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
        (
            "version",
            serde_json::json!("https://jsonfeed.org/version/1.1"),
        ),
    ]);
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}
