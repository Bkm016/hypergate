//! 控制台 ANSI 主题。
//!
//! @author sky

use std::borrow::Cow::{self, Borrowed, Owned};
use std::io::{self, IsTerminal};

/// 重置 ANSI 样式。
const RESET: &str = "\x1b[0m";

/// 加粗样式。
const BOLD: &str = "\x1b[1m";

/// 弱化样式。
const DIM: &str = "\x1b[2m";

/// 成功状态颜色。
const GREEN: &str = "\x1b[32m";

/// 错误状态颜色。
const RED: &str = "\x1b[31m";

/// 低强调边框颜色。
const BRIGHT_BLACK: &str = "\x1b[90m";

/// 为控制台输出添加统一 ANSI 样式。
pub fn colorize_output(rendered: &str) -> String {
    if !color_enabled() {
        return rendered.to_owned();
    }
    let lines = rendered.lines().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    for line in lines {
        if line.trim().is_empty() {
            output.push(String::new());
            continue;
        }
        if is_table_border(line) {
            output.push(paint(line, BRIGHT_BLACK));
            continue;
        }
        if is_table_row(line) {
            output.push(colorize_table_rules(line));
            continue;
        }
        if is_tree_line(line) {
            output.push(colorize_tree_line(line));
            continue;
        }
        if !line.starts_with(' ') {
            output.push(paint(line, BOLD));
            continue;
        }
        output.push(colorize_field_line(line));
    }
    output.join("\n")
}

/// 为交互提示符添加统一 ANSI 样式。
pub fn colorize_prompt<'a>(prompt: &'a str) -> Cow<'a, str> {
    if !color_enabled() {
        return Borrowed(prompt);
    }
    Owned(format!("{BOLD}{prompt}{RESET}"))
}

/// 为补全候选添加统一 ANSI 样式。
pub fn colorize_candidate<'a>(candidate: &'a str) -> Cow<'a, str> {
    if !color_enabled() {
        return Borrowed(candidate);
    }
    Owned(candidate.to_owned())
}

/// 判断当前输出是否适合写入 ANSI 颜色。
fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

/// 给面板字段行中的 value 部分着色。
fn colorize_field_line(line: &str) -> String {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let Some((key, value)) = split_key_value(body) else {
        return line.to_owned();
    };
    format!("{indent}{key}  {}", colorize_value(value))
}

/// 给 help 树的分支符和说明文本着色。
fn colorize_tree_line(line: &str) -> String {
    let Some(marker_start) = line.find("├─ ").or_else(|| line.find("└─ ")) else {
        return paint(line, BRIGHT_BLACK);
    };
    let marker_end = marker_start + "├─ ".len();
    let prefix = &line[..marker_end];
    let rest = &line[marker_end..];
    let Some((label, description)) = split_key_value(rest) else {
        return format!("{}{}", paint(prefix, BRIGHT_BLACK), rest);
    };
    format!(
        "{}{}  {}",
        paint(prefix, BRIGHT_BLACK),
        label,
        paint(description, DIM),
    )
}

/// 只弱化表格竖线,保留单元格文本原色。
fn colorize_table_rules(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    for ch in line.chars() {
        if ch == '│' {
            output.push_str(BRIGHT_BLACK);
            output.push(ch);
            output.push_str(RESET);
            continue;
        }
        output.push(ch);
    }
    output
}

/// 根据常见状态值选择颜色。
fn colorize_value(value: &str) -> String {
    let trimmed = value.trim();
    let style = match trimmed {
        "active" | "ok" => GREEN,
        "stopped" | "false" | "-" => BRIGHT_BLACK,
        value if value.starts_with("error") => RED,
        _ => RESET,
    };
    if style == RESET {
        return value.to_owned();
    }
    paint(value, style)
}

/// 按格式层约定的两个空格分隔 key 和 value。
fn split_key_value(value: &str) -> Option<(&str, &str)> {
    value
        .split_once("  ")
        .map(|(key, value)| (key.trim_end(), value.trim_start()))
}

/// 判断一行是否完全由表格边框字符组成。
fn is_table_border(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(is_table_rule)
}

/// 判断一行是否为表格数据行。
fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('│') && trimmed.ends_with('│')
}

/// 判断一行是否为 help 树节点。
fn is_tree_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("├─") || trimmed.starts_with("└─") || trimmed.starts_with("│")
}

/// 判断字符是否属于表格边框字符集。
fn is_table_rule(ch: char) -> bool {
    matches!(
        ch,
        '┌' | '┬' | '┐' | '├' | '┼' | '┤' | '└' | '┴' | '┘' | '─' | '│'
    )
}

/// 包裹 ANSI 样式并在末尾重置。
fn paint(value: &str, style: &str) -> String {
    format!("{style}{value}{RESET}")
}
