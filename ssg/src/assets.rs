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

/// 用 lightningcss 压缩一份 CSS；解析/压缩失败就原样返回，不让构建失败
fn minify_css(source: &str) -> String {
    use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};
    let Ok(mut sheet) = StyleSheet::parse(source, ParserOptions::default()) else {
        return source.to_string();
    };
    if sheet.minify(MinifyOptions::default()).is_err() {
        return source.to_string();
    }
    let opts = PrinterOptions {
        minify: true,
        ..Default::default()
    };
    match sheet.to_css(opts) {
        Ok(res) => res.code,
        Err(_) => source.to_string(),
    }
}

/// 用 minify-js 压缩一份 JS；解析失败或者压缩失败（这个 crate 对某些合法
/// 写法——比如三元表达式两支返回类型不对称——会内部断言 panic，不只是普通
/// Result::Err）都原样返回，不让构建失败。用 catch_unwind 兜底 panic。
fn minify_js(source: &str) -> String {
    use minify_js::{minify, Session, TopLevelMode};
    let result = std::panic::catch_unwind(|| {
        let session = Session::new();
        let mut out = Vec::with_capacity(source.len());
        match minify(&session, TopLevelMode::Global, source.as_bytes(), &mut out) {
            Ok(()) => Some(out),
            Err(_) => None,
        }
    });
    match result {
        Ok(Some(out)) => String::from_utf8(out).unwrap_or_else(|_| source.to_string()),
        _ => source.to_string(),
    }
}

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
        let raw = std::fs::read_to_string(entry.path())
            .with_context(|| format!("读取 {} 失败", entry.path().display()))?;
        let content = if !minify {
            raw
        } else if ext == "css" {
            minify_css(&raw)
        } else {
            minify_js(&raw)
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
