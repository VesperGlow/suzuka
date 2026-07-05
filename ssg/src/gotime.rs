//! Go time.Format 布局字符串的最小实现，覆盖本站 i18n 与模板里用到的形态。

use chrono::{DateTime, Datelike, FixedOffset, Timelike};

const MONTHS_LONG: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
const DAYS_LONG: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];

pub fn format(dt: &DateTime<FixedOffset>, layout: &str) -> String {
    // 按最长匹配优先的顺序扫描布局记号
    const TOKENS: [&str; 20] = [
        "2006", "January", "Jan", "Monday", "Mon", "Z07:00", "-07:00", "Z0700", "-0700", "15",
        "01", "02", "03", "04", "05", "06", "PM", "pm", "1", "2",
    ];
    let mut out = String::new();
    let bytes = layout.as_bytes();
    let mut i = 0;
    'outer: while i < bytes.len() {
        for tok in TOKENS {
            if layout[i..].starts_with(tok) {
                out.push_str(&render_token(dt, tok));
                i += tok.len();
                continue 'outer;
            }
        }
        let ch = layout[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn render_token(dt: &DateTime<FixedOffset>, tok: &str) -> String {
    let month = dt.month() as usize;
    match tok {
        "2006" => format!("{:04}", dt.year()),
        "06" => format!("{:02}", dt.year() % 100),
        "January" => MONTHS_LONG[month - 1].to_string(),
        "Jan" => MONTHS_LONG[month - 1][..3].to_string(),
        "Monday" => DAYS_LONG[dt.weekday().num_days_from_sunday() as usize].to_string(),
        "Mon" => DAYS_LONG[dt.weekday().num_days_from_sunday() as usize][..3].to_string(),
        "01" => format!("{:02}", dt.month()),
        "1" => dt.month().to_string(),
        "02" => format!("{:02}", dt.day()),
        "2" => dt.day().to_string(),
        "15" => format!("{:02}", dt.hour()),
        "03" => format!("{:02}", hour12(dt)),
        "3" => hour12(dt).to_string(),
        "04" => format!("{:02}", dt.minute()),
        "4" => dt.minute().to_string(),
        "05" => format!("{:02}", dt.second()),
        "5" => dt.second().to_string(),
        "PM" => if dt.hour() < 12 { "AM" } else { "PM" }.to_string(),
        "pm" => if dt.hour() < 12 { "am" } else { "pm" }.to_string(),
        "Z07:00" => offset_str(dt, true, true),
        "-07:00" => offset_str(dt, false, true),
        "Z0700" => offset_str(dt, true, false),
        "-0700" => offset_str(dt, false, false),
        _ => unreachable!(),
    }
}

fn hour12(dt: &DateTime<FixedOffset>) -> u32 {
    match dt.hour() % 12 {
        0 => 12,
        h => h,
    }
}

fn offset_str(dt: &DateTime<FixedOffset>, z_for_utc: bool, colon: bool) -> String {
    let secs = dt.offset().local_minus_utc();
    if secs == 0 && z_for_utc {
        return "Z".to_string();
    }
    let sign = if secs < 0 { '-' } else { '+' };
    let abs = secs.abs();
    let (h, m) = (abs / 3600, (abs % 3600) / 60);
    if colon {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}")
    }
}
