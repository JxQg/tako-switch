# Tako Switch v2.0.9 / v2.1.0 黑盒测试计划（Claude Desktop 全量 ASAR 汉化 + 跨平台权限引导与解耦）

> 版本顺延说明：本计划与双盲黑盒套件最初按冻结规格 `v2.0.9` 设计；因同日并行发版已占用 `v2.0.9` 与 `v2.0.10`，合流后统一随 **`v2.1.0`** 发布，套件内版本断言统一对齐 `env!("CARGO_PKG_VERSION")`（`2.1.0`）。

## 1. `v2.0.8` 基线差异验证（Baseline Differential Verification）

在合入 `v2.0.9` 功能分支前，已于独立测试工作树 `/tmp/tako-switch-wt-test-209`（基线提交 `348231e`，对应 `v2.0.8`）完成基线行为实测与代码契约核对：

| 验证维度 | `v2.0.8` 基线实测事实（`348231e`） | `v2.0.9` 冻结规格目标契约 |
|---|---|---|
| `Claude` UI 补丁状态感知 | `src-tauri/src/adapters/mod.rs:134-137` 中 `snapshot_app(&paths, AppId::Claude).ui_patch.status` 恒返回 `UiPatchStatus::NotApplicable`；实测运行 `cargo test --manifest-path /tmp/tako-switch-wt-test-209/src-tauri/Cargo.toml --test v208_i18n_and_plaza_blackbox`（`8 passed; 0 failed`，耗时 `0.79s`），`tc_v208_01` 确认 `Claude` 返回 `NotApplicable` | `snapshot_app(&paths, AppId::Claude)` 接入 `inspect_claude_ui_patch(paths)`，完整支持 `NotInstalled` / `Unpatched` / `Patched` / `OutdatedAfterUpgrade` / `Unsupported` 状态机 |
| 双包 ASAR 汉化与完整性头 | `AppPaths`（`src-tauri/src/config.rs:46-57`）仅含 `agy_app_override_dir`，无 `claude_app_override_dir` 与 `claude_i18n.rs`；未处理 `Claude` 的 `app.asar` + `ion-dist.asar` 双包白名单、51 个动态模型/思考键、`Info.plist` 双哈希及 Windows PE `ELECTRONASARINTEGRITY` | 纯 Rust 解析重打包 `app.asar` 与 `ion-dist.asar`，同步更新 macOS `Info.plist` 双 `ElectronAsarIntegrity` 哈希及 Windows PE（当 `EnableEmbeddedAsarIntegrityValidation == 1` 时）哈希、校验和与尾部证书，支持 100% 字节级无损还原 |
| 权限阻断分类与配置/补丁解耦 | `src-tauri/src/lib.rs:733-799` 中 `set_prefer_chinese_ui` 直接通过 `?` 传播 `patch_antigravity_ui_for_paths` 错误，无 `[TAKO_UI_PATCH_BLOCK]` 结构化错误分类，亦无 `apply_prefer_chinese_ui_for_paths` 解耦函数 | 引入 `UiPatchBlockKind` / `UiPatchPermissionPrompt` 跨平台分类；当 `.app` 目录受 macOS App Management 或 Windows UAC / 文件锁阻断时，配置层汉化（`Codex` / `Claude` / `Antigravity`）照常落盘成功，仅通过 `ui_patch_prompts` 返回非致命权限引导 |

---

## 2. 测试范围、双盲隔离原则与宿主机安全红线

- **双盲隔离边界**：
  - 全程仅在 `/tmp/tako-switch-wt-test-209`（分支 `test/v2.0.9-blackbox`）开展测试设计与实现，严禁读取开发工作树 `/tmp/tako-switch-wt-dev-209`，严禁修改主仓库 `/Users/bytedance/develop/tako-switch-v2`，严禁执行 `git push` 或创建 Git tag。
  - 所有黑盒断言严格对齐冻结契约 `v2.0.9-frozen-spec.md` 中声明的公开符号与数据契约，不依赖未公开私有实现细节。
- **宿主机与进程安全红线**：
  - 全部 Rust 集成测试强制使用 `tempfile::TempDir` + `AppPaths::for_test(tmp.path())`（断言 `paths.is_test == true`），并通过 `paths.claude_app_override_dir` 与 `paths.agy_app_override_dir` 将应用目录重定向至临时沙盒。
  - 严禁触碰宿主机 `/Applications/Claude.app`、`/Applications/Antigravity.app`、`~/.codex`、`~/.claude`、`~/.gemini`、`~/.tako` 或 `~/Library/Application Support/Claude*`。
  - `paths.is_test == true` 下严禁调用真实 `/usr/bin/codesign`、`/usr/bin/defaults`、`/usr/bin/osascript`、`/usr/bin/open` 或 Windows UAC 提权进程；严禁执行 `pkill -x tako-switch` 或 `killall tako-switch`。
  - Unix 只读权限沙盒测试（`chmod 0o555`）必须挂载 `Drop` 守卫（`ChmodRestoreGuard`），无论断言通过或 panic 均在 `TempDir` 析构前恢复 `0o755`，确保临时目录零残留清理。

---

## 3. 黑盒测试用例矩阵（`TC-209-01` .. `TC-209-12`）

| 用例编号 | 对应规格章节 | 输入前置条件 | 客观可判定预期（Pass/Fail Oracle） | 测试类型 |
|---|---|---|---|---|
| `TC-209-01` | §A.2 / §A.4 macOS `.app` 生命周期与快照接入 | `AppPaths::for_test` 沙盒：1) `claude_app_override_dir` 指向不存在路径；2) 指向缺失 `ion-dist.asar` 的残缺 `.app`；3) 指向完整合成 `Claude.app`（`6.28899.6`）并依次调用 `inspect_claude_ui_patch`、`patch_claude_ui_for_paths`、`snapshot_app` | 1) 未安装目录返回 `UiPatchStatus::NotInstalled`；2) 缺失任一 ASAR 返回 `UiPatchStatus::Unsupported`；3) 干净包初始返回 `UiPatchStatus::Unpatched`；4) 打补丁后返回 `UiPatchStatus::Patched`、`has_valid_backup == true`、`patch_version == Some("2.0.9")`，且 `snapshot_app(&paths, AppId::Claude).ui_patch.status == UiPatchStatus::Patched`；`Codex` / `DshTako` 仍为 `NotApplicable` | Blackbox Rust Integration |
| `TC-209-02` | §A.3 双包 ASAR 深度二进制与翻译/ICU 不变量 | 对合成 `Claude.app` 执行 `patch_claude_ui_for_paths` 后，按 Chromium Pickle 16B 头规范解析磁盘上的 `app.asar` 与 `ion-dist.asar` | 1) `app.asar`：注入 `__tako_i18n_patch.json`；`.vite/build/index.chunk-abc123.js` 含单次 `CLAUDE_PATCH_GUARD_COMMENT`、`QVe` 包含 `"zh-Hans":"zh-Hans"` 及 `zh-CN`、`sHe` 移除窄正则 `/[a-z]{2}-[A-Z]{2}/` 并放行 `zh-Hans.json`；`resources/i18n/zh-Hans.json` 保留原译文并补齐 8 个缺失键；2) `ion-dist.asar`：注入 `__tako_i18n_patch.json`；`shared-73-test.js`、`shared-107-test.js`（含逗号空格变体）、`c2c114516-test.js` 均含守卫注释且 11 语言数组追加 `"zh-Hans"`；显示名称表含 `"zh-Hans"` 与 `中文（简体）`；`i18n/dynamic/zh-Hans.json` 全量 51 键均翻译为简体中文；`i18n/zh-Hans.json` 与 `en-US.json` 键集合 100% 一致，Chat / Claude Code / Cowork / Settings 核心词条完成中文化，全部 `{var}` 与 `{count, plural, ...}` ICU 占位符集合及花括号配对零损坏；3) 两包中 `unpacked: true` 原生模块条目原样保留且无 `offset`，所有打包文件 4MB 分块 SHA-256 `integrity` 校验全部通过 | PE & ASAR Binary Invariant |
| `TC-209-03` | §A.4 macOS `Info.plist` 双包完整性哈希同步 | 提取打补丁后 `Contents/Info.plist` 中 `ElectronAsarIntegrity` 下 `Resources/app.asar` 与 `Resources/ion-dist.asar` 的 `hash` 字段 | 两处 `hash` 均已脱离打补丁前旧值，且分别与重打包后 `app.asar` Header JSON 字符串 SHA-256、`ion-dist.asar` Header JSON 字符串 SHA-256 精确一致（忽略大小写 64 位 hex） | PE & ASAR Binary Invariant |
| `TC-209-04` | §A.4 重复打补丁幂等与 5 文件字节级无损还原 | 在 `Patched` 状态下第 2 次调用 `patch_claude_ui_for_paths`，随后调用 `restore_claude_ui_for_paths`（并覆盖含 `MacOS_Claude.orig` / `CodeResources.orig` 模拟重签改动场景） | 1) 二次打补丁成功且 `backups/claude/ui_patch/{app.asar.orig,ion-dist.asar.orig}` 字节与最初干净包 100% 一致，JS 内 `CLAUDE_PATCH_GUARD_COMMENT` 计数仍为 `1`；2) 调用 `restore_claude_ui_for_paths` 后 `app.asar`、`ion-dist.asar`、`Info.plist`、`MacOS/Claude`、`CodeResources` 5 个文件全部 100% 字节级恢复至原始字节，`patch-meta.json` 已清理，状态回到 `UiPatchStatus::Unpatched` | Blackbox Rust Integration |
| `TC-209-05` | §A.4 客户端自升级检测与防降级安全 | 先对 `6.28899.6` 打补丁，随后分别模拟：1) 仅 `app.asar` 升级为干净 `6.29000.0`；2) 仅 `ion-dist.asar` 升级为干净 `6.29000.0`；3) 双包与 `Info.plist` 同步升级为干净 `6.29000.0` 并分别测试 `restore_claude_ui_for_paths` 与直接重打补丁 | 1) 任一或双包缺失补丁标识且存在 `patch-meta.json` 时，`inspect_claude_ui_patch` 均报告 `UiPatchStatus::OutdatedAfterUpgrade`；2) 在 `OutdatedAfterUpgrade` 下调用 `restore_claude_ui_for_paths` 绝不用 `6.28899.6` 旧备份覆盖 `6.29000.0` 文件，仅清理过期 `patch-meta.json` 并返回 `Unpatched`；3) 在 `OutdatedAfterUpgrade` 下直接调用 `patch_claude_ui_for_paths` 会将 `.orig` 刷新为干净 `6.29000.0`，再次还原可 100% 恢复 `6.29000.0` | Blackbox Rust Integration |
| `TC-209-06` | §A.2 / §A.4 / §B.1 Windows Squirrel 寻址、MSIX 拦截与 PE32+ 完整性 | 1) 构造 `AnthropicClaude/Claude.exe` stub 及 `app-6.9999.0`、`app-6.28000.0`、`app-6.28899.6` 多版本目录；2) 构造 `WindowsApps/AnthropicPBC.Claude_...` 目录；3) 构造含 Electron FuseV1 哨兵、双 64-hex 哈希、非零 `CheckSum`、`DataDirectory[4]` 与尾部 64B Authenticode 证书的合成 PE32+（`0x20b`）`claude.exe` | 1) Squirrel 按数值版本排序命中最高版 `app-6.28899.6`，低版本目录零改动；2) `WindowsApps` 路径 `inspect_claude_ui_patch` 返回 `Unsupported`（含中文说明），`patch_claude_ui_for_paths` 返回可解析为 `UiPatchBlockKind::WindowsMsixReadOnly` 的阻断错误；3) `inspect_electron_asar_integrity_fuse` 对无哨兵返回 `None`、`fuse=0` 返回 `Some(false)` 且 `patch_windows_pe_asar_integrity` 返回 `Ok(None)`（`claude.exe` 原样不动）；`fuse=1` 返回 `Some(true)`，打补丁更新双 64-hex 哈希、清零 `CheckSum` 与 `DataDirectory[4]`、截断尾部证书并写入 `Claude.exe.orig`，还原后 `claude.exe` 100% 字节级复原 | PE & ASAR Binary Invariant |
| `TC-209-07` | §B.1 跨平台 OS 权限错误分类与编解码往返 | 调用 `classify_io_error_for_os` 传入 macOS `raw_os_error(1)`/`13`、Windows `5`/`32` 及普通 `NotFound`；调用 `format_ui_patch_block_error` 与 `parse_ui_patch_block_error` | 1) `UI_PATCH_BLOCK_PREFIX == "[TAKO_UI_PATCH_BLOCK]"`；2) macOS `1`/`13` 映射为 `MacosAppManagement`（含系统设置引导文案），Windows `5` 映射为 `WindowsUac`，Windows `32` 映射为 `WindowsFileLock`，`NotFound` 返回 `None`；3) 编码结果无论直接传入还是被外层错误前缀/后缀包裹，`parse_ui_patch_block_error` 均精确还原原始 `UiPatchPermissionPrompt` | Permission Sandbox |
| `TC-209-08` | §B.2 只读受阻下配置层与 ASAR 补丁层解耦及重试 | Unix 下将沙盒 `Claude.app/Contents/Resources` 与 `Antigravity.app/Contents/Resources` 设为 `0o555`（挂载 `ChmodRestoreGuard`），调用 `apply_prefer_chinese_ui_for_paths(&paths, true, true, false)`；随后恢复 `0o755` 重试 | 1) 受阻调用返回 `Ok(prompts)`，内含 `Claude` 与 `Antigravity` 两条 `MacosAppManagement` 提示；2) `prefer_chinese_ui` 状态已持久化为 `true`，且 `Codex`（`config.toml` `localeOverride = "zh-CN"` + `AGENTS.md`）、`Claude`（`1p`/`3p` `config.json` `"locale": "zh-Hans"` + `~/.claude/settings.json` `"language": "Simplified Chinese"`）、`Antigravity`（`~/.gemini/AGENTS.md`）全部写入成功；3) 恢复 `0o755` 后重试返回空 `prompts`，双端 UI 补丁均收敛为 `Patched`；关闭汉化遇 `0o555` 时同样不回滚配置层清理 | Permission Sandbox |
| `TC-209-09` | §B.4 / §2.1 Tauri 命令与前端 API 绑定 | 检查 `src-tauri/src/lib.rs` `generate_handler!` 与 `src/lib/api.ts` 导出定义 | 注册 `patch_app_ui`、`restore_app_ui`、`open_macos_app_management_settings` 并保留旧命令兼容；`src/lib/api.ts` 导出 `UiPatchBlockKind`、`UiPatchPermissionPrompt`、`parseUiPatchBlockError`、`DashboardPayload.uiPatchPrompts` 及对应 `api.*` 方法 | Static & UI Verification |
| `TC-209-10` | §2.2 双端 UI 汉化按钮与权限引导弹窗契约 | 检查 `src/App.tsx`（及可选 `UiPatchPermissionModal.tsx`）DOM `data-testid` 与交互接线 | 同时渲染 `claude-ui-patch-btn`、`claude-ui-restore-btn`、`agy-ui-patch-btn`、`agy-ui-restore-btn`；权限引导弹窗具备 `ui-patch-permission-modal`、`open-app-management-settings-btn`、`retry-ui-patch-btn`、`dismiss-permission-modal-btn`，并明确提示配置级中文已生效 | Static & UI Verification |
| `TC-209-11` | §2.3 静态反篡改与测试标识门禁 | 运行 `python3 scripts/verify_client_mcp_ui.py` 与 `python3 scripts/verify_client_mcp_ui.py --self-test` | 受控 ASAR 访问仅允许出现在 `antigravity_i18n.rs`、`claude_i18n.rs`（及可选 `asar_patch.rs`），其余文件零违规；新增 `claude-ui-patch-btn` 与 `ui-patch-permission-modal` 纳入必填 `REQUIRED_TESTIDS`；无内部 MCP 标识符泄漏，自检与仓库扫描双通过 | Static & UI Verification |
| `TC-209-12` | §2.4 / §3 版本号同步与全量回归 | 核对 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`CURRENT_UI_PATCH_VERSION` 并运行 `npm run typecheck && npm run build && cargo test` | 四处版本号统一为 `"2.0.9"`；前端类型检查与构建零错误；`v208_i18n_and_plaza_blackbox` 与 `v209_claude_i18n_and_permissions_blackbox` 全部通过 | Blackbox Rust Integration |

---

## 4. 规格边界与消除歧义说明（Ambiguity Notes）

1. **`UiPatchInfo.target_asar_path` 在双 ASAR 架构下的取值范围**：
   - 冻结规格 §A.4 明确允许 `Claude` 在 `Patched` 等状态下将 `target_asar_path` 设为 `ion_asar_path` 或 `app_asar_path` 的显示字符串。黑盒套件不断言硬编码单一文件名，而是断言该路径位于目标安装实例目录（如 `Claude.app/Contents/Resources/` 或 Squirrel `app-6.28899.6/resources/`）且文件名为 `app.asar` 或 `ion-dist.asar`。
2. **JS 语言显示名称表 `中文（简体）` 的编码等价性**：
   - 冻结规格 §A.3 给出的注入示例为 `localName:"\u4E2D\u6587\uFF08\u7B80\u4F53\uFF09"`。在合法 JavaScript 产物中，直接输出 UTF-8 `"中文（简体）"` 与输出 `\u4E2D\u6587\uFF08\u7B80\u4F53\uFF09`（大小写十六进制）运行时完全等价。黑盒测试先将 JS 文本中的 `\uXXXX` 转义归一化解码后再断言包含 `"中文（简体）"`，避免因编码形式差异产生假阴性。
3. **`sHe()` 语言包文件名过滤正则的两种合法放行模式**：
   - 冻结规格 §A.3 允许将 `/[a-z]{2}-[A-Z]{2}/` 放宽为 `/^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*\.json$/` 或 `/[a-z]{2,3}-[A-Za-z0-9]{2,8}/`。黑盒套件断言原受限正则 `/[a-z]{2}-[A-Z]{2}/` 已被替换，且新正则包含 `[a-z]{2,3}` 与 `[A-Za-z0-9]{2,8}` 子标签规则，可同时覆盖 `zh-Hans.json`、`es-419.json` 与 `en-US.json`。
4. **测试态（`paths.is_test == true`）下 macOS 签名文件备份与还原覆盖**：
   - 冻结规格 §0.2 与 §A.4 规定 `paths.is_test == true` 时禁止调用真实 `/usr/bin/codesign`。为同时覆盖「常规测试态 `patch -> restore`」与「真实重签后 `MacOS/Claude` + `CodeResources` 被改写再执行 `restore`」两条路径，`TC-209-04` 先验证标准测试流下 5 个文件 100% 字节一致，再额外放置 `MacOS_Claude.orig` / `CodeResources.orig` 并改写目标包内对应文件后调用 `restore_claude_ui_for_paths`，实证还原逻辑对签名相关文件的字节级回写能力。
