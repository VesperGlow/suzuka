mod assets;
mod build;
mod config;
mod content;
mod diff;
mod gotime;
mod i18n;
mod images;
mod markdown;

use anyhow::{bail, Result};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("build") => {
            let source = flag_value(&args, "--source").unwrap_or_else(|| ".".into());
            let dest = flag_value(&args, "--dest").unwrap_or_else(|| "public-ssg".into());
            let minify = args.iter().any(|a| a == "--minify");
            build::build(&PathBuf::from(source), &PathBuf::from(dest), minify)
        }
        Some("diff") => {
            let positional: Vec<&String> =
                args[1..].iter().filter(|a| !a.starts_with("--")).collect();
            if positional.len() != 2 {
                bail!("用法: ssg diff <golden目录> <ssg产物目录>");
            }
            let mismatches =
                diff::diff_trees(&PathBuf::from(positional[0]), &PathBuf::from(positional[1]))?;
            if mismatches > 0 {
                bail!("{mismatches} 个文件与黄金基准不一致");
            }
            println!("对拍通过：产物与黄金基准一致（含已声明的归一化规则）");
            Ok(())
        }
        _ => bail!(
            "用法: ssg build [--source 目录] [--dest 目录] [--minify] | ssg diff <golden> <ours>"
        ),
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
