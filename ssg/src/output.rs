//! 写盘与产物收尾：HTML 压缩、CSP 内联脚本哈希收集、输出路径冲突检测。
//!
//! MINIFY / CSP_HASHES / WRITTEN 用 thread_local 而不是穿透传参：
//! 单进程单次构建，没有并发需求（见 begin 的说明）。

use anyhow::{Context, Result};
use minijinja::value::Value;
use minijinja::Environment;
use std::path::Path;

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

/// 一次构建的起点：设置 minify 开关，清空哈希与已写路径集合。
pub fn begin(minify: bool) {
    MINIFY.with(|m| m.set(minify));
    CSP_HASHES.with(|h| h.borrow_mut().clear());
    WRITTEN.with(|w| w.borrow_mut().clear());
}

/// 内联脚本哈希清单（每行一个 sha256-...），构建收尾写进 csp-hashes.txt。
pub fn csp_manifest() -> String {
    CSP_HASHES.with(|h| {
        let hashes = h.borrow();
        let mut out = String::new();
        for hash in hashes.iter() {
            out.push_str(hash);
            out.push('\n');
        }
        out
    })
}

pub fn render_to(
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

pub fn write_file(dest: &Path, rel: &str, content: &str) -> Result<()> {
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
