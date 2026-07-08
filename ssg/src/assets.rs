//! 资源管线：CSS/JS 压缩 + 指纹 + static/ 拷贝。
//!
//! 指纹散列跟 Hugo 不会一致（两边用的 minifier 不同，压缩结果逐字节有出入，
//! 散列自然也不同）——对拍时把 `.min.<hash>.<ext>` 里的散列归一化掉，见
//! diff.rs；命名规则本身兼容 backend/src/static_site.rs 的 immutable 缓存判断。

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// 用 lightningcss 压缩一份 CSS；解析/压缩失败就原样返回，不让构建失败，
/// 但会打警告——不然一次 CSS 语法错误会静默发布未压缩版本，没有任何信号。
fn minify_css(rel: &str, source: &str) -> String {
    use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};
    let mut sheet = match StyleSheet::parse(source, ParserOptions::default()) {
        Ok(sheet) => sheet,
        Err(err) => {
            eprintln!("警告: {rel} 压缩失败（解析），改用未压缩版本: {err}");
            return source.to_string();
        }
    };
    if let Err(err) = sheet.minify(MinifyOptions::default()) {
        eprintln!("警告: {rel} 压缩失败（minify），改用未压缩版本: {err}");
        return source.to_string();
    }
    let opts = PrinterOptions {
        minify: true,
        ..Default::default()
    };
    match sheet.to_css(opts) {
        Ok(res) => res.code,
        Err(err) => {
            eprintln!("警告: {rel} 压缩失败（输出），改用未压缩版本: {err}");
            source.to_string()
        }
    }
}

// JS 故意不压缩：minify-js 除了会对合法写法内部断言 panic（已用
// catch_unwind 兜底），还会产出不 panic 但真的跑错的代码——比如把一个
// 函数声明提到它引用的 const 所在的 if 块外面，导致函数体里那些变量
// 变成未定义，调用时直接 ReferenceError（关于页运行时长脚本就是这么
// 线上炸的：函数被提到 `if (uptime) { const c = ...; }` 外面，但函数体
// 里还在用 c）。这个 crate 的正确性信不过，先只做 HTML 空白压缩 + CSS
// 压缩，JS 原样发布。

pub struct Assets {
    /// 逻辑路径（如 "css/style.css"）→ 带指纹的发布路径（如 "/css/style.min.<hash>.css"）
    pub urls: HashMap<String, String>,
    /// theme.js 的内容，baseof 里内联
    pub theme_js: String,
}

pub fn build(source: &Path, dest: &Path, minify: bool) -> Result<Assets> {
    let mut urls = HashMap::new();
    let assets_dir = source.join("assets");
    for entry in WalkDir::new(&assets_dir) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&assets_dir)?
            .to_string_lossy()
            .replace('\\', "/");
        let ext = entry
            .path()
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        if ext != "css" && ext != "js" {
            continue;
        }
        // theme.js 只以内联形式进 baseof（见下方 theme_js），发布指纹副本
        // 没有任何引用，纯属产物噪音
        if rel == "js/theme.js" {
            continue;
        }
        let raw = std::fs::read_to_string(entry.path())
            .with_context(|| format!("读取 {} 失败", entry.path().display()))?;
        let content = if minify && ext == "css" {
            minify_css(&rel, &raw)
        } else {
            raw
        };
        let content = content.into_bytes();
        let hash = hex(&Sha256::digest(&content));
        let stem = rel.trim_end_matches(&format!(".{ext}"));
        let published = format!("/{stem}.min.{hash}.{ext}");
        let out_path = dest.join(published.trim_start_matches('/'));
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, &content)?;
        urls.insert(rel, published);
    }

    let theme_js = std::fs::read_to_string(assets_dir.join("js").join("theme.js"))
        .context("读取 assets/js/theme.js 失败")?
        .trim_end()
        .to_string();

    copy_tree(&source.join("static"), dest)?;
    Ok(Assets { urls, theme_js })
}

pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for entry in WalkDir::new(from) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(from)?;
        let target = to.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(entry.path(), &target)
            .with_context(|| format!("拷贝 {} 失败", entry.path().display()))?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
