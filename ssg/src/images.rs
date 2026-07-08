//! 文章配图的响应式变体：构建期把 bundle 里的大图缩成几档固定宽度，
//! 供 `<img srcset>` 按视口与 DPR 挑选；原图保留，做 srcset 最大档兼灯箱全尺寸。

use anyhow::{Context, Result};
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, Resizer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// srcset 档位。与 assets/css/style.css 的正文栏宽联动：768 覆盖手机 2x
/// 与 769–1023 区间的 1x；1280 覆盖桌面 1x 与手机 3x；1792 覆盖收起侧栏
/// 后变宽的正文 1x；更高 DPR 的宽屏直接落到原图档。
pub const VARIANT_WIDTHS: [u32; 3] = [768, 1280, 1792];

/// 有损重编码质量。变体是二次压缩，质量给到与截图原图相近的档位，
/// 避免叠加出可见的涂抹感。
const QUALITY: f32 = 82.0;

/// 一张图要生成哪些变体：只往小缩（放大没有信息量，纯浪费字节）。
/// 正文截图全是 webp；其他格式（若有）不动，原样发布。
pub fn variant_widths_for(name: &str, width: u32) -> Vec<u32> {
    if !name.ends_with(".webp") {
        return Vec::new();
    }
    VARIANT_WIDTHS
        .iter()
        .copied()
        .filter(|w| *w < width)
        .collect()
}

/// 变体文件名："001.webp" + 768 → "001.768w.webp"。宽度段带 `w` 后缀，
/// 与纯数字命名的相邻截图（002.webp）不会撞名。
pub fn variant_name(name: &str, width: u32) -> String {
    let stem = name.strip_suffix(".webp").unwrap_or(name);
    format!("{stem}.{width}w.webp")
}

/// srcset 候选串：全部缩放档位按宽度升序 + 原图收尾
/// （"…/001.768w.webp 768w, …, …/001.webp 2560w"）。
/// 无变体时返回 None，调用方退回单 src。正文 figure 与归档卡片封面共用。
pub fn srcset(bundle_rel: &str, name: &str, width: u32, variants: &[u32]) -> Option<String> {
    if variants.is_empty() {
        return None;
    }
    let mut candidates: Vec<String> = variants
        .iter()
        .map(|w| format!("{bundle_rel}{} {w}w", variant_name(name, *w)))
        .collect();
    candidates.push(format!("{bundle_rel}{name} {width}w"));
    Some(candidates.join(", "))
}

/// 一张原图的变体生成任务（widths 已按 variant_widths_for 过滤）。
pub struct ResizeJob {
    pub src: PathBuf,
    pub out_dir: PathBuf,
    pub name: String,
    pub widths: Vec<u32>,
}

/// 并行生成所有变体。每个任务解码一次原图、产出全部档位；
/// 解码/缩放/编码都是纯 CPU 活，用固定线程池摊平。
pub fn generate(jobs: &[ResizeJob]) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(jobs.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(i) else {
                            return Ok(());
                        };
                        run_job(job).with_context(|| {
                            format!("生成 {} 的缩放变体失败", job.src.display())
                        })?;
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("变体生成线程 panic")?;
        }
        Ok(())
    })
}

fn run_job(job: &ResizeJob) -> Result<()> {
    // 本地反复构建时产物目录不清空：变体比原图新就跳过重编码
    let pending: Vec<u32> = job
        .widths
        .iter()
        .copied()
        .filter(|w| is_stale(job, *w))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }

    let bytes = std::fs::read(&job.src)?;
    let decoded = webp::Decoder::new(&bytes)
        .decode()
        .context("webp 解码失败")?;
    let (width, height) = (decoded.width(), decoded.height());
    let pixel = if decoded.is_alpha() {
        PixelType::U8x4
    } else {
        PixelType::U8x3
    };
    let mut pixels = decoded.to_vec();
    let src_image =
        Image::from_slice_u8(width, height, &mut pixels, pixel).context("构造缩放输入失败")?;

    let mut resizer = Resizer::new();
    for target_w in pending {
        // 高度按原始宽高比取整，与 <img> 的 width/height 推导保持一致
        let target_h = ((u64::from(target_w) * u64::from(height) + u64::from(width) / 2)
            / u64::from(width))
        .max(1) as u32;
        let mut dst_image = Image::new(target_w, target_h, pixel);
        // 默认算法即 Lanczos3 卷积，缩小截图的质量足够
        resizer
            .resize(&src_image, &mut dst_image, None)
            .context("缩放失败")?;
        let encoded = match pixel {
            PixelType::U8x4 => {
                webp::Encoder::from_rgba(dst_image.buffer(), target_w, target_h).encode(QUALITY)
            }
            _ => webp::Encoder::from_rgb(dst_image.buffer(), target_w, target_h).encode(QUALITY),
        };
        let out_path = job.out_dir.join(variant_name(&job.name, target_w));
        std::fs::write(&out_path, &*encoded)
            .with_context(|| format!("写入 {} 失败", out_path.display()))?;
    }
    Ok(())
}

/// 变体缺失或比原图旧则需要重新生成。
fn is_stale(job: &ResizeJob, width: u32) -> bool {
    let out_path = job.out_dir.join(variant_name(&job.name, width));
    let (Ok(src_meta), Ok(out_meta)) = (job.src.metadata(), out_path.metadata()) else {
        return true;
    };
    match (src_meta.modified(), out_meta.modified()) {
        (Ok(src_time), Ok(out_time)) => out_time < src_time,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_only_shrink() {
        assert_eq!(variant_widths_for("001.webp", 2560), vec![768, 1280, 1792]);
        assert_eq!(variant_widths_for("001.webp", 1300), vec![768, 1280]);
        assert_eq!(variant_widths_for("001.webp", 768), Vec::<u32>::new());
        // 非 webp 与解析不出尺寸（width=0）的资源不做变体
        assert_eq!(variant_widths_for("001.png", 2560), Vec::<u32>::new());
        assert_eq!(variant_widths_for("001.webp", 0), Vec::<u32>::new());
    }

    #[test]
    fn variant_names() {
        assert_eq!(variant_name("001.webp", 768), "001.768w.webp");
        assert_eq!(
            variant_name("cover.final.webp", 1280),
            "cover.final.1280w.webp"
        );
    }
}
