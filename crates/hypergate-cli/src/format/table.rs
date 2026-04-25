//! 通用文本表格渲染器。

/// 表格列对齐方式。
#[derive(Clone, Copy)]
pub enum Align {
    /// 左对齐。
    Left,
    /// 右对齐。
    Right,
}

/// 表格列定义。
pub struct Column {
    /// 列标题。
    pub title: String,
    /// 对齐方式。
    pub align: Align,
}

/// 文本表格定义。
pub struct Table {
    /// 表格标题。
    pub title: String,
    /// 表格列定义。
    pub columns: Vec<Column>,
    /// 表格行。
    pub rows: Vec<Vec<String>>,
}

/// 渲染带标题、字段和表格的面板。
pub fn render_panel(title: &str, fields: Vec<(String, String)>, tables: Vec<Table>) -> String {
    let mut lines = Vec::new();
    let has_fields = !fields.is_empty();
    lines.push(title.to_owned());
    if has_fields {
        let key_width = fields.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
        for (key, value) in fields {
            lines.push(format!("  {key:<key_width$}  {value}"));
        }
    }
    if !tables.is_empty() {
        if has_fields {
            lines.push(String::new());
        }
        lines.push(render_tables(tables));
    }
    lines.join("\n")
}

/// 渲染一组表格。
pub fn render_tables(tables: Vec<Table>) -> String {
    tables
        .into_iter()
        .map(render_table)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 创建列定义。
pub fn column(title: &str, align: Align) -> Column {
    Column {
        title: title.to_owned(),
        align,
    }
}

/// 渲染单张表格,包含标题、边框、表头和数据行。
fn render_table(table: Table) -> String {
    let widths = table_widths(&table);
    let mut lines = vec![
        table.title,
        render_border('┌', '┬', '┐', &widths),
        render_row(
            &table
                .columns
                .iter()
                .map(|c| c.title.clone())
                .collect::<Vec<_>>(),
            &widths,
            &table.columns,
        ),
        render_border('├', '┼', '┤', &widths),
    ];
    for row in table.rows {
        lines.push(render_row(&row, &widths, &table.columns));
    }
    lines.push(render_border('└', '┴', '┘', &widths));
    lines.join("\n")
}

/// 根据表头和数据单元格计算每列宽度。
fn table_widths(table: &Table) -> Vec<usize> {
    let mut widths: Vec<_> = table
        .columns
        .iter()
        .map(|column| column.title.len())
        .collect();
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.len());
            }
        }
    }
    widths
}

/// 按列定义渲染一行数据。
fn render_row(row: &[String], widths: &[usize], columns: &[Column]) -> String {
    let cells = row
        .iter()
        .enumerate()
        .map(|(index, cell)| match columns[index].align {
            Align::Left => format!("{cell:<width$}", width = widths[index]),
            Align::Right => format!("{cell:>width$}", width = widths[index]),
        })
        .collect::<Vec<_>>();
    format!("│ {} │", cells.join(" │ "))
}

/// 根据列宽生成表格边框。
fn render_border(left: char, middle: char, right: char, widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .map(|width| "─".repeat(width + 2))
        .collect::<Vec<_>>();
    format!("{left}{}{right}", cells.join(&middle.to_string()))
}
