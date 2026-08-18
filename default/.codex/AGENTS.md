# AGENTS.md

本机环境：Windows，默认 shell 为 PowerShell。所有命令示例按 PowerShell 语法书写。

## PowerShell 使用要点

- 常用命令等价物：`Get-ChildItem`（列目录）、`Get-Content`（读文件）、`Get-Location`（当前目录）、`Set-Location`（切目录）、`Get-Process`/`Stop-Process`（进程管理）、`Get-Command`（查找命令）。
- 环境变量：读取用 `$env:NAME`（如 `$env:PATH`），设置用 `$env:NAME = "value"`，仅在当前会话生效；PATH 的路径分隔符是分号（`;`），修改时保留原有内容，不要整体覆盖。
- 写文件注意编码：`Out-File` / `Set-Content` 在 Windows PowerShell 下默认可能是 UTF-16，需要明确指定 `-Encoding utf8`。
- 字符串与引号：PowerShell 双引号内 `$var` 会展开、反引号 `` ` `` 是转义符；含空格路径用引号包裹。
- 管道与重定向：`|` 传递对象而不是纯文本，必要时用 `Select-Object`、`Format-Table` 格式化；避免依赖 Linux 风格的 `&&`/`||`（Windows PowerShell 5.1 不支持，改用分行或 PowerShell 7+ 语法）。
- 进程与后台：长时间运行或需要隐藏窗口时用 `Start-Process -WindowStyle Hidden`；不要用长时间阻塞等待。
- 删除/覆盖类操作：使用 `Remove-Item -LiteralPath`，先验证目标路径明确且在预期范围内，不执行针对宽泛目录的递归删除。

## 搜索工具（已安装并加入 PATH）

- `fd`：按文件名快速发现文件。示例：`fd settings src`、`fd -t f`（只看文件）、`fd -e ts src`（按扩展名）。默认遵循 .gitignore，跳过 node_modules、target、dist 等目录。
- `rg`：内容搜索（文本/正则）。示例：`rg -n "TODO" src`、`rg -n "someFunc" src-tauri`。默认遵循 .gitignore，需要搜隐藏/忽略文件时加 `--hidden`。
- `sg`（ast-grep，`sg` 为旧名）：语法感知的代码搜索。示例：`sg run -p '$FUNC(...)' -l rust src-tauri`（按 AST 模式匹配）、`sg run --kind <节点类型>`、`sg outline`（浏览符号/结构）；可用 `-C 3` 显示上下文、`--files-with-matches` 只列文件。无结果或结果不理想时回退 `rg`。

## 工作方式约定

- 查找文件或符号时先使用上述工具，不凭记忆猜测路径；搜索范围限定在相关目录。
- 在 PowerShell 下执行项目命令（如 npm/cargo 脚本）时，先按项目 README 或 package.json 确认命令，再原样执行。
- 破坏性命令（删除、覆盖、强制操作）执行前先做只读检查确认目标和范围。
- 不修改与当前任务无关的文件；改动后按项目既有测试/构建命令验证。