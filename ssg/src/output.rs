//! 写盘与产物收尾：构建目录发布、HTML 压缩、输出路径冲突检测。
//!
//! MINIFY / WRITTEN 用 thread_local 而不是穿透传参：
//! 单进程单次构建，没有并发需求（见 begin 的说明）。

use anyhow::{Context, Result};
use minijinja::value::Value;
use minijinja::Environment;
use std::path::{Path, PathBuf};

/// 构建成功后整目录发布，失败时保留上一份产物。
pub struct BuildOutput {
    staged: tempfile::TempDir,
    dest: PathBuf,
}

impl BuildOutput {
    pub fn new(source: &Path, dest: &Path) -> Result<Self> {
        let source = source.canonicalize().context("解析站点源码目录失败")?;
        let name = dest.file_name().context("输出目录必须有独立的目录名")?;
        let parent = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let parent = parent.canonicalize()?;
        let dest = parent.join(name);
        if source.starts_with(&dest)
            || [
                "content", "assets", "static", "i18n", "ssg", ".git", ".github", "scripts",
                "deploy",
            ]
            .iter()
            .any(|input| dest.starts_with(source.join(input)))
        {
            anyhow::bail!("输出目录不能覆盖源码或输入目录：{}", dest.display());
        }
        if let Ok(meta) = std::fs::symlink_metadata(&dest) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                anyhow::bail!("输出路径必须是普通目录：{}", dest.display());
            }
        }
        let staged = tempfile::Builder::new()
            .prefix(".ssg-build-")
            .tempdir_in(&parent)?;
        // TempDir 默认只有创建者能访问，发布目录需要允许静态服务器读取。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(staged.path(), std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(Self { staged, dest })
    }

    pub fn path(&self) -> &Path {
        self.staged.path()
    }

    pub fn publish(self) -> Result<()> {
        let backup = tempfile::Builder::new()
            .prefix(".ssg-previous-")
            .tempdir_in(self.dest.parent().expect("输出目录有父目录"))?;
        let previous = backup.path().join("site");
        let had_previous = self.dest.exists();
        if had_previous {
            std::fs::rename(&self.dest, &previous).context("暂存旧产物失败")?;
        }
        if let Err(err) = std::fs::rename(self.path(), &self.dest) {
            if had_previous {
                // 恢复失败时必须保留备份目录，不能让 TempDir 自动删掉上一份产物。
                if let Err(restore) = std::fs::rename(&previous, &self.dest) {
                    let saved = backup.keep();
                    anyhow::bail!(
                        "发布失败：{err}；恢复失败：{restore}；旧产物保存在 {}",
                        saved.display()
                    );
                }
            }
            return Err(err).context("发布产物失败，已保留旧产物");
        }
        Ok(())
    }
}

// 单进程单次构建，没有并发需求：用 thread_local 存 minify 开关，
// 免得给所有 write_file / render_to 调用点都加一个参数
thread_local! {
    static MINIFY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // 本次构建已写出的渲染产物路径。同一路径写两次一定是配置错误
    // （重复 slug、alias 撞真实页面、term 撞文章），谁后写谁赢会静默
    // 丢页面，必须当场报错。资源与图片走 assets.rs / fs::copy，文件名
    // 含指纹或按 bundle 目录隔离，不在此列。
    static WRITTEN: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

/// 一次构建的起点：设置 minify 开关，清空已写路径集合。
pub fn begin(minify: bool) {
    MINIFY.with(|m| m.set(minify));
    WRITTEN.with(|w| w.borrow_mut().clear());
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
    std::fs::write(&path, bytes).with_context(|| format!("写入 {} 失败", path.display()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishing_removes_stale_files_and_aborted_build_keeps_previous_output() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let dest = source.join("public");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("obsolete.html"), "old").unwrap();
        {
            let aborted = BuildOutput::new(&source, &dest).unwrap();
            std::fs::write(aborted.path().join("partial.html"), "partial").unwrap();
        }
        assert!(dest.join("obsolete.html").exists());
        assert!(!dest.join("partial.html").exists());

        let output = BuildOutput::new(&source, &dest).unwrap();
        std::fs::write(output.path().join("index.html"), "new").unwrap();
        output.publish().unwrap();
        assert!(!dest.join("obsolete.html").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("index.html")).unwrap(),
            "new"
        );
    }

    #[test]
    fn refuses_to_replace_source_or_its_inputs() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        for dest in [
            source.clone(),
            root.path().to_path_buf(),
            source.join("content"),
            source.join("ssg/output"),
        ] {
            assert!(
                BuildOutput::new(&source, &dest).is_err(),
                "{}",
                dest.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_output() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("public");
        std::os::unix::fs::symlink(root.path(), &dest).unwrap();
        assert!(BuildOutput::new(root.path(), &dest).is_err());
    }
}
