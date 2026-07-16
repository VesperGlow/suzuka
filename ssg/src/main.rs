mod assets;
mod build;
mod config;
mod content;
mod feeds;
mod gotime;
mod i18n;
mod images;
mod markdown;
mod meta;
mod output;
mod serve;
mod sitemap;

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
        Some("serve") => {
            let source = flag_value(&args, "--source").unwrap_or_else(|| ".".into());
            let dest = flag_value(&args, "--dest").unwrap_or_else(|| "public-dev".into());
            let addr = flag_value(&args, "--addr").unwrap_or_else(|| "127.0.0.1:1313".into());
            serve::serve(&PathBuf::from(source), &PathBuf::from(dest), &addr)
        }
        _ => bail!(
            "用法: ssg build [--source 目录] [--dest 目录] [--minify]\n      ssg serve [--source 目录] [--dest 目录] [--addr 地址:端口]"
        ),
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
