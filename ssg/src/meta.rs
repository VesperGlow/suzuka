//! head 元数据块：og/twitter/microdata、JSON-LD、redirectTo 跳转块、
//! 别名桩 HTML，以及全站共用的 HTML 转义工具。

use crate::config::{Language, SiteConfig};

pub(crate) struct InternalMeta<'a> {
    pub(crate) permalink: &'a str,
    pub(crate) site_title: &'a str,
    pub(crate) title: &'a str,
    pub(crate) description: &'a str,
    pub(crate) locale: String,
    pub(crate) og_type: &'a str,
    pub(crate) section: Option<&'a str>,
    pub(crate) date_published: &'a str,
    pub(crate) date_modified: &'a str,
    pub(crate) tags: &'a [String],
    pub(crate) image: String,
    pub(crate) word_count: Option<usize>,
    pub(crate) jsonld: Option<&'a str>,
}

impl<'a> InternalMeta<'a> {
    /// og:type=website、无 tags/jsonld 的基础形态；文章/独立页在此之上覆写
    /// og_type / section / tags / image / word_count / jsonld
    pub(crate) fn new(
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
pub(crate) fn internal_meta_block(m: &InternalMeta) -> String {
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
pub(crate) fn schema_blogposting_json(
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
    root.insert(
        "description".into(),
        json!(crate::build::collapse_ws(description)),
    );
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
pub(crate) fn alias_html(locale: &str, target: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"{locale}\">\n\t<head>\n\t\t<title>{target}</title>\n\t\t<link rel=\"canonical\" href=\"{target}\">\n\t\t<meta charset=\"utf-8\">\n\t\t<meta http-equiv=\"refresh\" content=\"0; url={target}\">\n\t</head>\n</html>\n"
    )
}

/// redirectTo 存在时算出 (canonical 绝对地址, meta-refresh+JS 跳转整块)
pub(crate) fn redirect_fields(
    config: &SiteConfig,
    redirect_to: Option<&str>,
) -> Option<(String, String)> {
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

pub(crate) fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#x27;")
}
