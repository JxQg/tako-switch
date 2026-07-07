# Tako Switch 操作文档

本文面向使用 Tako Switch 配置 Codex / Claude Code 的用户。正常流程不需要手动编辑配置文件。

## 使用前准备

请先确认已经安装需要配置的客户端：

- Codex：命令行里可以运行 `codex`。
- Claude Code：命令行里可以运行 `claude`。
- Tako 账号：用于获取或授权 ApiKey。

打开 Tako Switch 后，首页会自动检测 Codex 和 Claude Code 的本机状态。如果显示未检测到，可以点击右上角的刷新按钮重新检测。

## 推荐流程

1. 打开 Tako Switch。
2. 在“首页”确认 Codex / Claude Code 的检测状态。
3. 点击“登录 Tako”，在系统浏览器中完成授权。
4. 授权完成后，浏览器会通过 `takoswitch://` 回到 Tako Switch。
5. 在弹出的导入窗口里确认要配置的目标：Codex、Claude Code，或两者都选。
6. 点击“生成预览”，检查将要写入的配置。
7. 确认无误后点击“应用配置”。
8. 如果配置了 Codex，请新开一个终端再运行 Codex。

也可以进入“导入配置”页，手动粘贴 ApiKey 后生成预览并应用。

## 页面说明

### 首页

首页用于快速判断当前状态：

- “本机客户端状态”显示 Codex / Claude Code 是否可用。
- Tako 账号区域显示登录状态、套餐、用量和模型摘要。
- “登录 Tako”会打开系统浏览器完成授权。
- “导入配置”会打开一个更轻量的导入窗口。

### 导入配置

导入配置页是主要写入入口：

- 勾选 Codex / Claude Code，决定本次写入哪些客户端。
- 填写或确认网关地址、ApiKey 和模型。
- 点击“生成预览”查看写入前后差异。
- 点击“应用配置”真正写入文件和环境变量。

### 当前配置

当前配置页是只读视图，用于确认现有配置文件位置和内容。这里不会写入文件。

## 字段说明

### Codex OpenAI 兼容地址

默认值是：

```text
https://tako.shiroha.tech/v1
```

Codex 通过 OpenAI 兼容接口访问 Tako，因此这里需要包含 `/v1`。

### Claude Code 网关地址

默认值是：

```text
https://tako.shiroha.tech
```

Claude Code 的地址不要包含 `/v1`。如果填写成 `https://tako.shiroha.tech/v1`，应用会提示修正。

### API Key / Token

可以通过 Tako 登录自动填充，也可以手动粘贴。界面会默认隐藏密钥，预览中也会遮罩显示。

### Codex 模型

选择 Codex 时必须填写模型。登录 Tako 后，如果能读取到模型列表，应用会优先从列表中选择 OpenAI / Codex 可用模型。

### Claude 模型

Claude 模型可以留空。留空时，Claude Code 会继续使用自己的默认模型。

## 写入内容

### Codex

Tako Switch 会写入：

- 配置文件：`$CODEX_HOME/config.toml`。
- 如果没有设置 `CODEX_HOME`，则写入 `~/.codex/config.toml`。
- 用户环境变量：`TAKO_CODEX_API_KEY`。

写入后的 Codex 配置会设置模型、模型服务商和 Tako 网关地址。ApiKey 不会直接写进 `config.toml`，而是通过环境变量读取。

在 Windows 上，环境变量会写入当前用户的环境变量注册表；在 macOS / Linux 上，会写入 shell 配置文件。应用后请新开终端，让新环境变量生效。

### Claude Code

Tako Switch 会写入：

- 配置文件：`~/.claude/settings.json`。
- `env.ANTHROPIC_BASE_URL`：Claude Code 网关地址。
- `env.ANTHROPIC_AUTH_TOKEN`：ApiKey。
- `env.ANTHROPIC_MODEL`：可选模型；留空时会移除此项。

如果原来的 `settings.json` 里还有其他字段，应用会尽量保留。

## 预览、备份与恢复

点击“生成预览”时，应用只计算将要写入的内容，不会修改文件。

点击“应用配置”时，应用会先备份原文件，再写入新配置：

- Codex 备份位于程序目录下的 `backups/codex/`。
- Claude Code 备份位于程序目录下的 `backups/claude-code/`。
- 最近一次写入结果记录在程序目录下的 `tako-config-backups.json`。

写入完成后，“结果与恢复”区域会显示配置路径和备份路径。点击“恢复”可以把对应文件恢复到写入前状态。

如果写入前文件不存在，恢复时会删除由 Tako Switch 创建的新配置文件。

## 常见问题

### 已安装 Codex / Claude Code，但首页显示未检测到

请先新开一个终端确认命令是否能运行：

```bash
codex --version
claude --version
```

如果终端也无法识别命令，请检查安装路径是否加入了 `PATH`。如果终端可以识别，回到 Tako Switch 点击刷新重新检测。

### 应用配置后 Codex 仍然没有使用新 ApiKey

Codex 从环境变量读取密钥。应用配置后请关闭旧终端，重新打开一个终端再运行 Codex。

### Tako 登录没有回到应用

可以回到 Tako Switch 的“导入配置”页，手动粘贴 ApiKey。Tako Switch 不会读取浏览器 Cookie，也不会抓取网页内容。

### Claude Code 地址提示不能包含 `/v1`

这是正常校验。Codex 使用 OpenAI 兼容地址，需要 `/v1`；Claude Code 使用 Anthropic 网关根地址，不要带 `/v1`。

### 预览里看不到完整 ApiKey

这是安全处理。密钥会在预览和当前配置视图里遮罩显示。

## 安全说明

- 应用不会在未确认预览的情况下静默写入配置。
- Codex 的 ApiKey 通过用户环境变量保存，不直接写入 Codex 配置文件。
- Claude Code 的 ApiKey 会写入 `settings.json` 的 `env` 字段，这是 Claude Code 读取认证信息的方式。
- 当前配置视图和预览会对密钥做遮罩处理。
