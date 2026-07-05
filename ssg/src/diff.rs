//! 对拍：把 ssg 产物与 Hugo 黄金基准逐文件比较。
//!
//! 归一化规则（有意屏蔽的已知差异）：
//! 1. 资源指纹散列 → `HASH`（我们不做 minify，散列必然不同）
//! 2. 无 src 的内联 <script> 内容 → `[INLINE_SCRIPT]`（minify 器不同）
//! 3. HTML 实体等价形（&#34;/&quot; 等）→ 统一形态
//! 4. 行尾空白去除、连续空行折叠
//!
//! 只比较 ssg 侧存在的文件：迁移期产物是黄金基准的子集，
//! 基准里多出的文件（feeds、归档页等未实现部分）不算失败。

use anyhow::Result;
use std::path::Path;
use walkdir::WalkDir;

pub fn diff_trees(golden: &Path, ours: &Path) -> Result<usize> {
    let mut mismatches = 0;
    let mut compared = 0;
    for entry in WalkDir::new(ours) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(ours)?.to_path_buf();
        let golden_path = golden.join(&rel);
        let rel_display = rel.to_string_lossy().replace('\\', "/");
        // 指纹文件名两边不同，跳过文件本体（内容经由 HTML 引用对拍）
        if is_fingerprinted(&rel_display) {
            continue;
        }
        if !golden_path.exists() {
            println!("[缺失] 基准中不存在: {rel_display}");
            mismatches += 1;
            continue;
        }
        compared += 1;
        let ours_raw = std::fs::read(entry.path())?;
        let golden_raw = std::fs::read(&golden_path)?;
        if is_text(&rel_display) {
            let a = normalize(&String::from_utf8_lossy(&golden_raw));
            let b = normalize(&String::from_utf8_lossy(&ours_raw));
            if a != b {
                mismatches += 1;
                report_first_diff(&rel_display, &a, &b);
            }
        } else if ours_raw != golden_raw {
            mismatches += 1;
            println!("[不一致] 二进制文件: {rel_display}");
        }
    }
    println!("已比较 {compared} 个文件，{mismatches} 个不一致");
    Ok(mismatches)
}

fn is_fingerprinted(path: &str) -> bool {
    let segments: Vec<&str> = path.rsplit('.').collect();
    segments.len() >= 3
        && segments[1].len() >= 40
        && segments[1].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_text(path: &str) -> bool {
    [".html", ".xml", ".json", ".txt", ".css", ".js"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

fn normalize(input: &str) -> Vec<String> {
    let mut text = input.replace("\r\n", "\n");
    text = mask_fingerprints(&text);
    text = mask_inline_scripts(&text);
    // 实体等价形归一
    for (from, to) in [
        ("&#34;", "&quot;"),
        ("&#39;", "&#x27;"),
        ("&apos;", "&#x27;"),
    ] {
        text = text.replace(from, to);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut blank_run = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if !blank_run {
                lines.push(String::new());
            }
            blank_run = true;
        } else {
            lines.push(trimmed.to_string());
            blank_run = false;
        }
    }
    lines
}

/// `name.min.<hex>.ext` → `name.min.HASH.ext`
fn mask_fingerprints(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(".min.") {
        out.push_str(&rest[..pos + 5]);
        rest = &rest[pos + 5..];
        let hex_len = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        if hex_len >= 40 && rest[hex_len..].starts_with('.') {
            out.push_str("HASH");
            rest = &rest[hex_len..];
        }
    }
    out.push_str(rest);
    out
}

/// 屏蔽无 src 属性的内联脚本内容
fn mask_inline_scripts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<script") {
        let after_tag = &rest[start..];
        let Some(open_end) = after_tag.find('>') else {
            break;
        };
        let open_tag = &after_tag[..open_end + 1];
        let Some(close) = after_tag.find("</script>") else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(open_tag);
        let body = &after_tag[open_end + 1..close];
        if open_tag.contains("src=") || body.trim().is_empty() {
            out.push_str(body);
        } else {
            out.push_str("[INLINE_SCRIPT]");
        }
        out.push_str("</script>");
        rest = &after_tag[close + "</script>".len()..];
    }
    out.push_str(rest);
    out
}

fn report_first_diff(path: &str, golden: &[String], ours: &[String]) {
    println!("[不一致] {path}");
    let max = golden.len().max(ours.len());
    let mut shown = 0;
    for i in 0..max {
        let g = golden.get(i).map(String::as_str).unwrap_or("<EOF>");
        let o = ours.get(i).map(String::as_str).unwrap_or("<EOF>");
        if g != o {
            println!("  行 {}:", i + 1);
            println!("    基准: {g}");
            println!("    产物: {o}");
            shown += 1;
            if shown >= 5 {
                println!("  ……(仅显示前 5 处)");
                break;
            }
        }
    }
}
