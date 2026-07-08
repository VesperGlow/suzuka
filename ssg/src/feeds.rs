//! 订阅与索引输出：RSS、JSON Feed、index.json、guestbook-posts.json。
//! 条目数据来自 build.rs 聚合好的 PostCard，这里只负责序列化格式。

use crate::build::PostCard;
use crate::meta::escape_attr;
use chrono::{DateTime, Datelike, FixedOffset};

/// RSS 单个条目
pub(crate) struct RssItem<'a> {
    pub(crate) title: &'a str,
    pub(crate) link: &'a str,
    pub(crate) pub_date: &'a str,
    /// RSS `<category>`：term 的自动 title
    pub(crate) categories: &'a [String],
    /// 完整正文（站内根相对 src/href 会改成绝对地址后再 HTML 转义）
    pub(crate) content_html: Option<&'a str>,
}

/// 首页 RSS 的条目是全站内容（文章 + about/guestbook 独立页），所以单独存
/// 一份带日期、拥有所有权的列表，按日期混排后再借出 RssItem
pub(crate) struct OwnedRssItem {
    pub(crate) title: String,
    pub(crate) link: String,
    pub(crate) pub_date: String,
    pub(crate) date: DateTime<FixedOffset>,
    pub(crate) categories: Vec<String>,
    pub(crate) content_html: String,
}

/// RSS channel 元数据
pub(crate) struct RssChannel<'a> {
    pub(crate) title: String,
    pub(crate) link: String,
    pub(crate) description: &'a str,
    pub(crate) site_title: &'a str,
    pub(crate) locale: &'a str,
    pub(crate) author: &'a str,
    pub(crate) last_build_date: Option<String>,
    pub(crate) self_link: String,
    pub(crate) base_url: &'a str,
}

/// 文章卡片 → RSS 条目
pub(crate) fn card_rss_items<'a>(cards: &[&'a PostCard]) -> Vec<RssItem<'a>> {
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

pub(crate) fn render_rss(ch: &RssChannel, items: &[RssItem]) -> String {
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
pub(crate) fn channel_title(page_title: &str, site_title: &str) -> String {
    if page_title == site_title || page_title.is_empty() {
        site_title.to_string()
    } else {
        format!("{page_title} · {site_title}")
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
pub(crate) fn index_json(items: &[&PostCard], has_cjk: bool) -> String {
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
pub(crate) fn guestbook_posts_json(items: &[GuestbookItem]) -> String {
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

pub(crate) struct GuestbookItem {
    pub(crate) title: String,
    pub(crate) rel: String,
    pub(crate) date_iso: String,
}

/// feed.json（JSON Feed 1.1）
#[allow(clippy::too_many_arguments)]
pub(crate) fn feed_json(
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
