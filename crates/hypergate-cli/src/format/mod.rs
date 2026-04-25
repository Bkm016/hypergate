//! 终端输出格式化工具。

mod table;
mod theme;

pub use table::{Align, Column, Table, column, render_panel, render_tables};
pub use theme::{colorize_candidate, colorize_output, colorize_prompt};
