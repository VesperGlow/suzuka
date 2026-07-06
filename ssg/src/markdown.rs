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
    let mut first_image_src = None;
    let mut used_ids: HashMap<String, usize> = HashMap::new();
    let mut in_code = false;
    // goldmark typographer 的引号配对计数器（&ldquo;/&rdquo; 开合）整篇
    // 文档共享，不按 block 重置——包括图片的 alt/caption，也计进同一个计数
    // 器。但 flanking 判定用的「前一个字符」是按 block 走的：段落、标题、
    // 独立图片各自有自己的 text.Reader，开头都视为「前面是空白」，所以这几
    // 类 block 开始处都要把 prev_char 重置成 None，quotes 计数器不受影响。
    let mut quotes = QuoteState::default();
    let mut prev_char: Option<char> = None;

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
                        // 图片是独立 block：flanking 用的「前一个字符」重置
                        // （跟段落/标题一样），但引号配对计数器继续跟全文共享
                        // ——alt/caption 的引号也会计进去，跟正文一样。
                        prev_char = None;
                        let (figure, src, _alt_plain) = build_figure(
                            inner,
                            resources,
                            bundle_rel,
                            &mut prev_char,
                            &mut quotes,
                        );
                        if first_image_src.is_none() {
                            first_image_src = Some(src);
                        }
                        out.push(Event::Html(CowStr::from(figure)));
                        i = end + 1;
                        continue;
                    }
                }
                prev_char = None;
                out.push(events[i].clone());
            }
            Event::Start(Tag::Image { .. }) => {
                // 行内图片（与文字混排）：仍走 figure hook，但保留所在段落，
                // 引号状态（flanking 基准 + 配对计数器）跟周围文字连续
                let end = find_end(&events, i, |e| matches!(e, Event::End(TagEnd::Image)))
                    .unwrap_or(i);
                let (figure, src, _alt_plain) = build_figure(
                    &events[i..=end],
                    resources,
                    bundle_rel,
                    &mut prev_char,
                    &mut quotes,
                );
                if first_image_src.is_none() {
                    first_image_src = Some(src);
                }
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
                prev_char = None;
                let inner_html = render_inline(inner, &mut prev_char, &mut quotes);
                let level_num = heading_level_num(level);
                if matches!(level, HeadingLevel::H2 | HeadingLevel::H3) {
                    toc.push(TocEntry {
                        level: level_num,
                        id: id.clone(),
                        inner_html: inner_html.clone(),
                    });
                }
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
                if in_code {
                    out.push(events[i].clone());
                } else {
                    out.push(Event::Html(CowStr::from(typograph_escape(
                        text,
                        &mut prev_char,
                        &mut quotes,
                    ))));
                }
            }
            // 正文里出现的原始 HTML/JS（block 或 inline）一律当纯文本转义输出，
            // 不再直接执行——对齐 Hugo goldmark `unsafe=false` 的行为，
            // 避免文章正文意外粘入的 <script>/事件属性被原样发布到线上。
            Event::Html(text) | Event::InlineHtml(text) => {
                out.push(Event::Text(text.clone()));
            }
            _ => out.push(events[i].clone()),
        }
        i += 1;
    }

    let mut html_out = String::new();
    html::push_html(&mut html_out, out.into_iter());
    // wordCount 与 Hugo 一致：对最终渲染出的 HTML 去标签统计（tpl.StripHTML），
    // 而非渲染过程中手工攒的文本——这样图片 alt（属性，Hugo 不计入）与
    // figcaption（可见文本，Hugo 计入）的取舍就自动跟 Hugo 对齐了。
    let plain = strip_html(&html_out);
    Rendered {
        html: html_out,
        toc,
        plain,
        first_image_src,
    }
}

/// 复刻 Hugo `tpl.StripHTML`：去标签统计词数用，保留实体原样不解码。
fn strip_html(html: &str) -> String {
    if !html.contains('<') && !html.contains('>') {
        return html.to_string();
    }
    let normalized = html
        .replace('\n', " ")
        .replace("</p>", "\n")
        .replace("<br>", "\n")
        .replace("<br />", "\n");
    let mut out = String::with_capacity(normalized.len());
    let mut in_tag = false;
    for ch in normalized.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
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
    prev_char: &mut Option<char>,
    quotes: &mut QuoteState,
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
    // alt/caption 的行内内容也是 goldmark 同一遍 inline parse 的一部分，
    // 引号配对状态照样跟正文共享同一个计数器
    let alt = typograph_escape(&alt_plain, prev_char, quotes);

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
        format!(
            "<figcaption>{}</figcaption>",
            typograph_escape(&title, prev_char, quotes)
        )
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
fn render_inline(events: &[Event], prev_char: &mut Option<char>, quotes: &mut QuoteState) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            Event::Text(t) => out.push_str(&typograph_escape(t, prev_char, quotes)),
            Event::Code(t) => {
                out.push_str("<code>");
                out.push_str(&escape_text(t));
                out.push_str("</code>");
                *prev_char = t.chars().last().or(*prev_char);
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

/// goldmark typographer 引号配对计数：按 block 作用域，开一个记一个、配对关一个
#[derive(Default)]
struct QuoteState {
    double: i32,
    single: i32,
}

/// 文字节点：HTML 转义 + goldmark typographer 的实体输出。
///
/// 引号的开/合判定复刻 goldmark `extension/typographer.go`：用 CommonMark
/// 强调分隔符那套 left-/right-flanking 规则判断某个引号能不能当开引号、能不能
/// 当合引号——两边都是「词」字符（比如中文不带空格的场景）时两者都成立，视为
/// 歧义，goldmark 会原样放过，最终被当成普通文本转义成 `&quot;`。这也是为什么
/// 中文行文里大量引号最终是 `&quot;` 而不是 `&ldquo;`/`&rdquo;` 的原因。
fn typograph_escape(text: &str, prev_char: &mut Option<char>, quotes: &mut QuoteState) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => {
                let after = run_end_after(&chars, i, ch);
                let (can_open, can_close) = quote_flanking(*prev_char, after);
                if can_open && !can_close {
                    out.push_str("&ldquo;");
                    quotes.double += 1;
                } else if quotes.double > 0 && can_close && !can_open {
                    out.push_str("&rdquo;");
                    quotes.double -= 1;
                } else {
                    out.push_str("&quot;");
                }
            }
            '\'' => {
                let after = run_end_after(&chars, i, ch);
                // don't/it's/Suzuka's：缩略或所有格，前后都是字母数字，直接当撇号
                if prev_char.map(|p| p.is_alphanumeric()).unwrap_or(false)
                    && after.map(|a| a.is_alphabetic()).unwrap_or(false)
                {
                    out.push_str("&rsquo;");
                } else {
                    let (can_open, can_close) = quote_flanking(*prev_char, after);
                    if can_open && !can_close {
                        out.push_str("&lsquo;");
                        quotes.single += 1;
                    } else if after.map(|a| !a.is_ascii_digit()).unwrap_or(true) {
                        // 复数所有格/口语缩略（Smiths'、doin'）：goldmark 这里不看
                        // counter，只要紧跟的不是数字就当撇号处理
                        out.push_str("&rsquo;");
                        if quotes.single > 0 {
                            quotes.single -= 1;
                        }
                    } else if quotes.single > 0 && can_close && !can_open {
                        out.push_str("&rsquo;");
                        quotes.single -= 1;
                    } else {
                        out.push_str("&#39;");
                    }
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
        *prev_char = Some(ch);
        i += 1;
    }
    out
}

/// goldmark `parser.ScanDelimiter` 的一个关键细节：判定 flanking 用的
/// 「after」不是紧邻的下一个字符，而是跳过当前位置起、所有与 ch 相同的连续
/// 字符之后的第一个不同字符——`""` 这种连续两个引号会被当一个游程扫描。
fn run_end_after(chars: &[char], i: usize, ch: char) -> Option<char> {
    let mut j = i + 1;
    while j < chars.len() && chars[j] == ch {
        j += 1;
    }
    chars.get(j).copied()
}

/// CommonMark 强调分隔符的 left-/right-flanking 判定，套用到单个引号字符上：
/// (can_open, can_close)。文本片段边界（before/after 缺失）按空白处理。
fn quote_flanking(before: Option<char>, after: Option<char>) -> (bool, bool) {
    let after_space = after.map(is_typo_space).unwrap_or(true);
    let after_punct = after.map(is_typo_punct).unwrap_or(false);
    let before_space = before.map(is_typo_space).unwrap_or(true);
    let before_punct = before.map(is_typo_punct).unwrap_or(false);
    let can_open = !after_space && (!after_punct || before_space || before_punct);
    let can_close = !before_space && (!before_punct || after_space || after_punct);
    (can_open, can_close)
}

fn is_typo_space(ch: char) -> bool {
    ch.is_whitespace()
}

/// 近似 Go unicode.IsPunct（Unicode P* 类别）：ASCII 精确匹配标点类别（排除
/// $ + < = > ^ ` | ~ 这些 Symbol 类别字符），非 ASCII 覆盖本站行文常见的
/// CJK/全角标点。
fn is_typo_punct(ch: char) -> bool {
    if ch.is_ascii() {
        matches!(
            ch,
            '!' | '"'
                | '#'
                | '%'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | ','
                | '-'
                | '.'
                | '/'
                | ':'
                | ';'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '_'
                | '{'
                | '}'
        )
    } else {
        matches!(
            ch,
            '。' | '，'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{2018}'
                | '\u{2019}'
                | '《'
                | '》'
                | '（'
                | '）'
                | '【'
                | '】'
                | '「'
                | '」'
                | '『'
                | '』'
                | '—'
                | '\u{2013}'
                | '…'
                | '～'
                | '·'
        )
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
