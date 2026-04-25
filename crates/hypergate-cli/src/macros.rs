//! 命令注册宏。
//!
//! @author sky

/// 创建一条 `RegisteredCommand`。
///
/// 默认值面向业务 version app: 可在控制台调用、不启用参数补全、
/// 显示在 help 树里。`usage` 默认从 `path` 生成,带参数的命令可以
/// 显式覆盖。
#[macro_export]
macro_rules! hypergate_command {
    (
        $first:ident $($path:ident)*, $description:literal,
        |$context:ident| $body:block
        $(, usage: $usage:literal)?
        $(, scope: $scope:ident)?
        $(, completion: $completion:ident)?
        $(, arguments: $arguments:ident)?
        $(, visible: $visible:expr)?
        $(,)?
    ) => {{
        /// 内联命令 handler,由 `hypergate_command!` 在调用点生成。
        fn __hypergate_inline_command(
            $context: $crate::command::CommandContext<'_>,
            _args: &[&str],
        ) -> $crate::command::CommandResult {
            $body
        }
        $crate::hypergate_command! {
            $first $($path)* => __hypergate_inline_command, $description
            $(, usage: $usage)?
            $(, scope: $scope)?
            $(, completion: $completion)?
            $(, arguments: $arguments)?
            $(, visible: $visible)?
        }
    }};
    (
        $first:ident $($path:ident)*, $description:literal,
        |$context:ident, $args:ident| $body:block
        $(, usage: $usage:literal)?
        $(, scope: $scope:ident)?
        $(, completion: $completion:ident)?
        $(, arguments: $arguments:ident)?
        $(, visible: $visible:expr)?
        $(,)?
    ) => {{
        /// 内联命令 handler,由 `hypergate_command!` 在调用点生成。
        fn __hypergate_inline_command(
            $context: $crate::command::CommandContext<'_>,
            $args: &[&str],
        ) -> $crate::command::CommandResult {
            $body
        }
        $crate::hypergate_command! {
            $first $($path)* => __hypergate_inline_command, $description
            $(, usage: $usage)?
            $(, scope: $scope)?
            $(, completion: $completion)?
            $(, arguments: $arguments)?
            $(, visible: $visible)?
        }
    }};
    (
        $first:ident $($path:ident)* => $handler:path, $description:literal
        $(, usage: $usage:literal)?
        $(, scope: $scope:ident)?
        $(, completion: $completion:ident)?
        $(, arguments: $arguments:ident)?
        $(, visible: $visible:expr)?
        $(,)?
    ) => {
        $crate::command::RegisteredCommand {
            path: &[stringify!($first), $(stringify!($path)),*],
            usage: $crate::__hypergate_command_ident_usage!([$first $($path)*] $($usage)?),
            description: $description,
            scope: $crate::__hypergate_command_scope!($($scope)?),
            completion: $crate::__hypergate_command_completion!($($completion)?),
            arguments: $crate::__hypergate_command_arguments!($($arguments)?),
            visible: $crate::__hypergate_command_visible!($($visible)?),
            handler: $handler,
        }
    };
    (
        path: [$first:literal $(, $path:literal)* $(,)?],
        $(usage: $usage:literal,)?
        description: $description:literal,
        handler: $handler:path
        $(, scope: $scope:ident)?
        $(, completion: $completion:ident)?
        $(, arguments: $arguments:ident)?
        $(, visible: $visible:expr)?
        $(,)?
    ) => {
        $crate::command::RegisteredCommand {
            path: &[$first, $($path),*],
            usage: $crate::__hypergate_command_usage!([$first $(, $path)*] $($usage)?),
            description: $description,
            scope: $crate::__hypergate_command_scope!($($scope)?),
            completion: $crate::__hypergate_command_completion!($($completion)?),
            arguments: $crate::__hypergate_command_arguments!($($arguments)?),
            visible: $crate::__hypergate_command_visible!($($visible)?),
            handler: $handler,
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_ident_usage {
    ([$first:ident $($path:ident)*]) => {
        concat!(stringify!($first), $(" ", stringify!($path)),*)
    };
    ([$first:ident $($path:ident)*] $usage:literal) => {
        $usage
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_usage {
    ([$first:literal $(, $path:literal)*]) => {
        concat!($first, $(" ", $path),*)
    };
    ([$first:literal $(, $path:literal)*] $usage:literal) => {
        $usage
    };
}

/// 创建命令列表。
///
/// 该宏适合直接传给 `VersionAppConsoleBuilder::commands`。列表中的每个
/// 条目都使用 `hypergate_command!` 的字段格式。
#[macro_export]
macro_rules! hypergate_commands {
    ($($first:ident $($path:ident)*, $description:literal, |$context:ident| $body:block $(, $key:ident : $value:tt)* ;)+) => {
        &[$($crate::hypergate_command! {
            $first $($path)*, $description, |$context| $body $(, $key: $value)*
        }),+]
    };
    ($($first:ident $($path:ident)*, $description:literal, |$context:ident, $args:ident| $body:block $(, $key:ident : $value:tt)* ;)+) => {
        &[$($crate::hypergate_command! {
            $first $($path)*, $description, |$context, $args| $body $(, $key: $value)*
        }),+]
    };
    ($($first:ident $($path:ident)* => $handler:path, $description:literal $(, $key:ident : $value:tt)* ;)+) => {
        &[$($crate::hypergate_command! {
            $first $($path)* => $handler, $description $(, $key: $value)*
        }),+]
    };
    ($({ $($command:tt)* }),+ $(,)?) => {
        &[$($crate::hypergate_command! { $($command)* }),+]
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_arguments {
    () => {
        $crate::command::CommandArguments::None
    };
    (None) => {
        $crate::command::CommandArguments::None
    };
    (Any) => {
        $crate::command::CommandArguments::Any
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_scope {
    () => {
        $crate::command::CommandScope::Console
    };
    (Both) => {
        $crate::command::CommandScope::Console
    };
    (Console) => {
        $crate::command::CommandScope::Console
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_completion {
    () => {
        $crate::command::CompletionKind::None
    };
    (None) => {
        $crate::command::CompletionKind::None
    };
    (Version) => {
        $crate::command::CompletionKind::Version
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hypergate_command_visible {
    () => {
        true
    };
    ($visible:expr) => {
        $visible
    };
}
