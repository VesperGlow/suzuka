//! 资源管线：CSS/JS 指纹 + static/ 拷贝。
//!
//! 与 Hugo 的差异（有意为之，对拍时归一化指纹散列）：
//! 暂不做真正的 minify，只保留 `<名字>.min.<sha256>.<扩展名>` 的产物命名，
//! 该命名兼容 backend/src/static_site.rs 的 immutable 缓存判断。

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

pub struct Assets {
    /// 逻辑路径（如 "css/style.css"）→ 带指纹的发布路径（如 "/css/style.min.<hash>.css"）
    pub urls: HashMap<String, String>,
    /// theme.js 的内容，baseof 里内联
    pub theme_js: String,
}

pub fn build(source: &Path, dest: &Path) -> Result<Assets> {
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
        let content = std::fs::read(entry.path())?;
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
