//! 订阅与索引输出：RSS、JSON Feed、index.json。
//! 条目数据来自 build.rs 聚合好的 PostCard，这里只负责序列化格式。

use crate::build::PostCard;
use crate::meta::escape_attr;
use chrono::Datelike;

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

/// RSS channel 元数据
pub(crate) struct RssChannel<'a> {
    pub(crate) title: String,
    pub(crate) link: String,
    pub(crate) description: &'a str,
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
    s.push_str(&format!("    <language>{}</language>\n", ch.locale));
    s.push_str(&format!(
        "    <copyright>© {} {}</copyright>\n",
        chrono::Utc::now().year(),
        escape_attr(ch.author)
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

/// index.json：文章标题、标签、日期和正文摘要组成的静态搜索索引。
pub(crate) fn index_json(items: &[&PostCard], has_cjk: bool) -> String {
    let items: Vec<_> = items
        .iter()
        .map(|post| {
            serde_json::json!({
                "content": truncate_runes(&post.plain, 800, has_cjk),
                "date": post.date_display,
                "tags": post.tags_raw,
                "title": post.title,
                "url": post.rel,
            })
        })
        .collect();
    serde_json::json!({"items": items}).to_string()
}

/// JSON Feed 1.1，作者与站点共用同一名称。
pub(crate) fn feed_json(
    site_title: &str,
    home_url: &str,
    feed_url: &str,
    description: &str,
    locale: &str,
    items: &[&PostCard],
    base_url: &str,
) -> String {
    let items: Vec<_> = items
        .iter()
        .map(|post| {
            let content_html = post
                .content_html
                .replace("src=\"/", &format!("src=\"{base_url}/"))
                .replace("href=\"/", &format!("href=\"{base_url}/"));
            let mut item = serde_json::json!({
                "content_html": content_html,
                "date_modified": post.date_rfc3339,
                "date_published": post.date_rfc3339,
                "id": post.permalink,
                "tags": post.tags_raw,
                "title": post.title,
                "url": post.permalink,
            });
            if let Some(image) = &post.image {
                item["image"] = serde_json::json!(image);
            }
            if !post.summary.is_empty() {
                item["summary"] = serde_json::json!(post.summary);
            }
            item
        })
        .collect();
    let feed = serde_json::json!({
        "authors": [{"name": site_title}],
        "description": description,
        "feed_url": feed_url,
        "home_page_url": home_url,
        "items": items,
        "language": locale,
        "title": site_title,
        "version": "https://jsonfeed.org/version/1.1",
    });
    serde_json::to_string_pretty(&feed).expect("JSON Feed 只包含可序列化的字符串和数组")
}
