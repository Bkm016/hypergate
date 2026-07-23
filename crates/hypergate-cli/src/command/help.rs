//! 通用帮助和错误输出指令。

use hypergate_core::HyperResult;

use crate::command::{CommandContext, CommandFuture, CommandOutput, RegisteredCommand};
use crate::format::render_panel;

/// 输出当前注册表的帮助树。
pub fn help<'a>(context: CommandContext<'a>, args: &'a [&'a str]) -> CommandFuture<'a> {
    Box::pin(async move {
        Ok(CommandOutput {
            summary: "console=help".to_owned(),
            rendered: command_help(context.registry.commands(), args),
        })
    })
}

/// 输出指定命令路径下的帮助树。
pub fn scoped_help(
    context: CommandContext<'_>,
    _args: &[&str],
    path: &[&str],
) -> HyperResult<CommandOutput> {
    Ok(CommandOutput {
        summary: "console=help".to_owned(),
        rendered: command_help(context.registry.commands(), path),
    })
}

/// 输出退出提示。
pub fn exit<'a>(_context: CommandContext<'a>, _args: &'a [&'a str]) -> CommandFuture<'a> {
    Box::pin(async move {
        Ok(CommandOutput {
            summary: "console=quit".to_owned(),
            rendered: render_panel(
                "Console",
                vec![(
                    "message".to_owned(),
                    "press Ctrl+C to stop this process".to_owned(),
                )],
                Vec::new(),
            ),
        })
    })
}

/// 生成统一错误输出。
pub fn error_output(message: &str) -> CommandOutput {
    CommandOutput {
        summary: format!("error={message}"),
        rendered: render_panel(
            "Error",
            vec![("message".to_owned(), message.to_owned())],
            Vec::new(),
        ),
    }
}

/// 从当前注册表生成指定路径下的命令树。
pub(super) fn command_help(commands: &[RegisteredCommand], path: &[&str]) -> String {
    let mut nodes = Vec::new();
    for command in commands
        .iter()
        .filter(|command| command.scope.allows_console())
        .filter(|command| command.visible)
        .filter(|command| path_matches(command.path, path))
    {
        insert_command(&mut nodes, command, 0);
    }
    render_tree(&command_title(path), &nodes)
}

/// 判断命令路径是否属于当前 help 查询范围。
fn path_matches(command_path: &[&str], path: &[&str]) -> bool {
    if path.is_empty() {
        return true;
    }
    command_path.len() >= path.len() && command_path.iter().zip(path.iter()).all(|(a, b)| a == b)
}

/// 为根 help 和子树 help 生成统一标题。
fn command_title(path: &[&str]) -> String {
    if path.is_empty() {
        return "Command Console".to_owned();
    }
    format!("Commands: {}", path.join(" "))
}

#[derive(Clone)]
struct HelpNode {
    /// 当前树节点显示的命令片段。
    label: String,
    /// 叶子节点显示的命令说明。
    description: String,
    /// 子命令节点。
    children: Vec<HelpNode>,
}

impl HelpNode {
    /// 创建一个暂不绑定说明文本的树节点。
    fn new(label: String, description: String) -> Self {
        Self {
            label,
            description,
            children: Vec::new(),
        }
    }
}

/// 渲染完整 help 树。
fn render_tree(title: &str, nodes: &[HelpNode]) -> String {
    let label_width = tree_label_width(nodes);
    let mut lines = vec![title.to_owned()];
    for (index, node) in nodes.iter().enumerate() {
        let last = index + 1 == nodes.len();
        render_node(node, "", last, label_width, &mut lines);
    }
    lines.join("\n")
}

/// 渲染单个节点和它的子树。
fn render_node(
    node: &HelpNode,
    prefix: &str,
    last: bool,
    label_width: usize,
    lines: &mut Vec<String>,
) {
    let branch = if last { "└─ " } else { "├─ " };
    lines.push(format!(
        "{prefix}{branch}{:<label_width$}  {}",
        node.label, node.description,
    ));
    let child_prefix = if last {
        format!("{prefix}   ")
    } else {
        format!("{prefix}│  ")
    };
    for (index, child) in node.children.iter().enumerate() {
        let child_last = index + 1 == node.children.len();
        render_node(child, &child_prefix, child_last, label_width, lines);
    }
}

/// 计算整棵树的最大标签宽度,保证说明列对齐。
fn tree_label_width(nodes: &[HelpNode]) -> usize {
    nodes
        .iter()
        .map(|node| node.label.len().max(tree_label_width(&node.children)))
        .max()
        .unwrap_or(0)
}

/// 把扁平命令路径插入 help 树。
fn insert_command(nodes: &mut Vec<HelpNode>, command: &RegisteredCommand, depth: usize) {
    let label = command_label(command, depth);
    let is_leaf = depth + 1 == command.path.len();
    let index = match nodes.iter().position(|node| node.label == label) {
        Some(index) => index,
        None => {
            nodes.push(HelpNode::new(label, String::new()));
            nodes.len() - 1
        }
    };
    if is_leaf {
        nodes[index].description = command.description.to_owned();
        return;
    }
    insert_command(&mut nodes[index].children, command, depth + 1);
}

/// 叶子节点优先使用 usage 尾段,展示 `<version>` 这类参数占位。
fn command_label(command: &RegisteredCommand, depth: usize) -> String {
    if depth + 1 != command.path.len() {
        return command.path[depth].to_owned();
    }
    let usage_parts = command.usage.split_whitespace().collect::<Vec<_>>();
    if usage_parts.len() <= depth {
        return command.path[depth].to_owned();
    }
    usage_parts[depth..].join(" ")
}
