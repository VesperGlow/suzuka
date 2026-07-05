//! Markdown 渲染：pulldown-cmark + 与 goldmark/Hugo 对齐的定制处理。
//!
//! 对齐点：
//! - typographer：ASCII 引号/破折号/省略号转 &rsquo; &ldquo; &ndash; &hellip; 等实体
//! - 标题自动 ID（GitHub 风格，CJK 保留、标点去除、重复加 -1 后缀）
//! - render-image hook：<figure> 包裹、bundle 资源尺寸、懒加载属性
//! - render-link hook：外链加 target="_blank" rel="noopener noreferrer"
//! - 独立成段的图片不包 <p>（wrapStandAloneImageWithinParagraph=false）

use crate::content::Resource;
use pulldown_cmark::{html, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

pub struct TocEntry {
    pub level: u32,
    pub id: String,
    pub inner_html: String,
}

pub struct Rendered {
    pub html: String,
    pub toc: Vec<TocEntry>,
    pub plain: String,
    pub first_image_src: Option<String>,
}

pub fn render(body: &str, resources: &[Resource], bundle_rel: &str) -> Rendered {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let events: Vec<Event> = Parser::new_ext(body, options).collect();
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    let mut toc = Vec::new();
    let mut plain = String::new();
    let mut first_image_src = None;
    let mut used_ids: HashMap<String, usize> = HashMap::new();
    let mut in_code = false;

    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Paragraph) => {
                // 段落里只有一张图片时：去掉 <p> 包裹，图片按块级 figure 输出
                if let Some(end) = find_end(&events, i, |e| {
                    matches!(e, Event::End(TagEnd::Paragraph))
                }) {
                    let inner = &events[i + 1..end];
                    if is_standalone_image(inner) {
                        let (figure, src, alt_plain) =
                            build_figure(inner, resources, bundle_rel);
                        if first_image_src.is_none() {
                            first_image_src = Some(src);
                        }
                        plain.push_str(&alt_plain);
                        plain.push('\n');
                        out.push(Event::Html(CowStr::from(figure)));
                        i = end + 1;
                        continue;
                    }
                }
                out.push(events[i].clone());
            }
            Event::Start(Tag::Image { .. }) => {
                // 行内图片（与文字混排）：仍走 figure hook，但保留所在段落
                let end = find_end(&events, i, |e| matches!(e, Event::End(TagEnd::Image)))
                    .unwrap_or(i);
                let (figure, src, alt_plain) =
                    build_figure(&events[i..=end], resources, bundle_rel);
                if first_image_src.is_none() {
                    first_image_src = Some(src);
                }
                plain.push_str(&alt_plain);
                out.push(Event::InlineHtml(CowStr::from(figure)));
                i = end + 1;
                continue;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level;
                let end = find_end(&events, i, |e| {
                    matches!(e, Event::End(TagEnd::Heading(_)))
                })
                .unwrap_or(i);
                let inner = &events[i + 1..end];
                let raw_text = collect_plain(inner);
                let id = dedupe_id(auto_heading_id(&raw_text), &mut used_ids);
                let inner_html = render_inline(inner);
                let level_num = heading_level_num(level);
                if matches!(level, HeadingLevel::H2 | HeadingLevel::H3) {
                    toc.push(TocEntry {
                        level: level_num,
                        id: id.clone(),
                        inner_html: inner_html.clone(),
                    });
                }
                plain.push_str(&raw_text);
                plain.push('\n');
                let tag = format!("<{level} id=\"{id}\">{inner_html}</{level}>\n");
                out.push(Event::Html(CowStr::from(tag)));
                i = end + 1;
                continue;
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                let mut a = format!("<a href=\"{}\"", escape_attr(dest_url));
                if !title.is_empty() {
                    a.push_str(&format!(" title=\"{}\"", escape_attr(title)));
                }
                if dest_url.starts_with("http") {
                    a.push_str(" target=\"_blank\" rel=\"noopener noreferrer\"");
                }
                a.push('>');
                out.push(Event::InlineHtml(CowStr::from(a)));
            }
            Event::End(TagEnd::Link) => {
                out.push(Event::InlineHtml(CowStr::from("</a>")));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                out.push(events[i].clone());
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push(events[i].clone());
            }
            Event::Text(text) => {
                plain.push_str(text);
                if in_code {
                    out.push(events[i].clone());
                } else {
                    out.push(Event::Html(CowStr::from(typograph_escape(text))));
                }
            }
            Event::Code(text) => {
                plain.push_str(text);
                out.push(events[i].clone());
            }
            Event::SoftBreak | Event::HardBreak => {
                plain.push(' ');
                out.push(events[i].clone());
            }
            Event::End(TagEnd::Paragraph) => {
                plain.push('\n');
                out.push(events[i].clone());
            }
            _ => out.push(events[i].clone()),
        }
        i += 1;
    }

    let mut html_out = String::new();
    html::push_html(&mut html_out, out.into_iter());
    Rendered {
        html: html_out,
        toc,
        plain,
        first_image_src,
    }
}

/// Hugo .TableOfContents 的 HTML 形态（两层：h2 / h3）
pub fn toc_html(entries: &[TocEntry]) -> String {
    let mut out = String::from("<nav id=\"TableOfContents\">\n  <ul>\n");
    let mut idx = 0;
    while idx < entries.len() {
        let entry = &entries[idx];
        if entry.level != 2 {
            idx += 1;
            continue;
        }
        let children: Vec<&TocEntry> = entries[idx + 1..]
            .iter()
            .take_while(|e| e.level == 3)
            .collect();
        if children.is_empty() {
            out.push_str(&format!(
                "    <li><a href=\"#{}\">{}</a></li>\n",
                entry.id, entry.inner_html
            ));
        } else {
            out.push_str(&format!(
                "    <li><a href=\"#{}\">{}</a>\n      <ul>\n",
                entry.id, entry.inner_html
            ));
            for child in &children {
                out.push_str(&format!(
                    "        <li><a href=\"#{}\">{}</a></li>\n",
                    child.id, child.inner_html
                ));
            }
            out.push_str("      </ul>\n    </li>\n");
        }
        idx += 1 + children.len();
    }
    out.push_str("  </ul>\n</nav>");
    out
}

/// pulldown-cmark 的 HeadingLevel 是 0 基枚举，转成 1..6 的层级号
fn heading_level_num(level: HeadingLevel) -> u32 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn find_end<F: Fn(&Event) -> bool>(events: &[Event], from: usize, pred: F) -> Option<usize> {
    events[from + 1..]
        .iter()
        .position(pred)
        .map(|p| from + 1 + p)
}

fn is_standalone_image(inner: &[Event]) -> bool {
    if inner.is_empty() {
        return false;
    }
    let starts_image = matches!(inner.first(), Some(Event::Start(Tag::Image { .. })));
    let ends_image = matches!(inner.last(), Some(Event::End(TagEnd::Image)));
    starts_image && ends_image && {
        // 中间不允许出现图片之外的兄弟节点
        let mut depth = 0;
        for e in inner {
            match e {
                Event::Start(Tag::Image { .. }) => depth += 1,
                Event::End(TagEnd::Image) => depth -= 1,
                _ if depth == 0 => return false,
                _ => {}
            }
        }
        true
    }
}

/// 从图片事件序列构造 figure HTML（复刻 render-image.html 的空白痕迹）
fn build_figure(
    image_events: &[Event],
    resources: &[Resource],
    bundle_rel: &str,
) -> (String, String, String) {
    let (mut dest, mut title) = (String::new(), String::new());
    if let Some(Event::Start(Tag::Image {
        dest_url,
        title: t,
        ..
    })) = image_events.first()
    {
        dest = dest_url.to_string();
        title = t.to_string();
    }
    let alt_events = &image_events[1..image_events.len().saturating_sub(1)];
    let alt_plain = collect_plain(alt_events);
    let alt = typograph_escape(&alt_plain);

    let resource = resources.iter().find(|r| r.name == dest);
    let title_attr = if title.is_empty() {
        String::new()
    } else {
        format!(" title=\"{}\"", escape_attr(&title))
    };
    let (img, src) = match resource {
        Some(res) => {
            let src = format!("{bundle_rel}{}", res.name);
            (
                format!(
                    "<img src=\"{src}\" alt=\"{alt}\" width=\"{}\" height=\"{}\" loading=\"lazy\" decoding=\"async\"{title_attr}>",
                    res.width, res.height
                ),
                src,
            )
        }
        None => (
            format!(
                "<img src=\"{}\" alt=\"{alt}\" loading=\"lazy\" decoding=\"async\"{title_attr}>",
                escape_attr(&dest)
            ),
            dest.clone(),
        ),
    };
    let caption = if title.is_empty() {
        String::new()
    } else {
        format!("<figcaption>{}</figcaption>", typograph_escape(&title))
    };
    let figure = format!("<figure>\n  {img}\n  {caption}\n</figure>\n");
    (figure, src, alt_plain)
}

fn collect_plain(events: &[Event]) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            Event::Text(t) | Event::Code(t) => out.push_str(t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out
}

/// 渲染标题内部的行内内容（文字走 typographer，行内代码保留 <code>）
fn render_inline(events: &[Event]) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            Event::Text(t) => out.push_str(&typograph_escape(t)),
            Event::Code(t) => {
                out.push_str("<code>");
                out.push_str(&escape_text(t));
                out.push_str("</code>");
            }
            Event::Start(Tag::Emphasis) => out.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</em>"),
            Event::Start(Tag::Strong) => out.push_str("<strong>"),
            Event::End(TagEnd::Strong) => out.push_str("</strong>"),
            _ => {}
        }
    }
    out
}

/// goldmark autoHeadingID（GitHub 风格）：小写、保留字母数字与连字符、空格转连字符、其余去除
fn auto_heading_id(text: &str) -> String {
    let mut id = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                id.push(lc);
            }
        } else if ch == ' ' || ch == '\t' || ch == '-' {
            id.push('-');
        } else if ch == '_' {
            id.push('_');
        }
        // 其余标点丢弃
    }
    if id.is_empty() {
        "heading".to_string()
    } else {
        id
    }
}

fn dedupe_id(id: String, used: &mut HashMap<String, usize>) -> String {
    let count = used.entry(id.clone()).or_insert(0);
    let result = if *count == 0 {
        id.clone()
    } else {
        format!("{id}-{count}")
    };
    *count += 1;
    result
}

/// 文字节点：HTML 转义 + goldmark typographer 的实体输出
fn typograph_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut prev: Option<char> = None;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => {
                if is_open_context(prev) {
                    out.push_str("&ldquo;");
                } else {
                    out.push_str("&rdquo;");
                }
            }
            '\'' => {
                if prev.map(|p| p.is_alphanumeric()).unwrap_or(false) {
                    out.push_str("&rsquo;");
                } else if is_open_context(prev) {
                    out.push_str("&lsquo;");
                } else {
                    out.push_str("&rsquo;");
                }
            }
            '.' if i + 2 < chars.len() && chars[i + 1] == '.' && chars[i + 2] == '.' => {
                out.push_str("&hellip;");
                i += 2;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                if i + 2 < chars.len() && chars[i + 2] == '-' {
                    out.push_str("&mdash;");
                    i += 2;
                } else {
                    out.push_str("&ndash;");
                    i += 1;
                }
            }
            _ => out.push(ch),
        }
        prev = Some(ch);
        i += 1;
    }
    out
}

fn is_open_context(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(p) => p.is_whitespace() || matches!(p, '(' | '[' | '{' | '-' | '—'),
    }
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
