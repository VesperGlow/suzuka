mod assets;
mod build;
mod config;
mod content;
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
        _ => bail!("用法: ssg build [--source 目录] [--dest 目录] [--minify]"),
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
