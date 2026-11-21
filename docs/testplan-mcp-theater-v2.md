# Tako Switch V2 灵动内联剧场与现场试跑黑盒测试计划

## 1. 测试范围与隔离原则

- **测试职责**：仅依据需求规格与基线公开契约独立编写黑盒测试计划、Rust 后端集成测试与 Python 前端/资源/安全校验脚本；严禁读取或依赖开发分支实现代码。
- **沙盒隔离**：Rust 集成测试统一使用 `AppPaths::for_test(temp_dir)`（`is_test = true`）及本地 `tokio::net::TcpListener` 零外部依赖 Mock Tako HTTP Server，严禁发起真实外网扣费请求或触碰宿主机配置。
- **安全与回归基线**：严禁反编译或修改任何第三方客户端 `app.asar`；确保原有 `chinese_localization_blackbox.rs`（12 条汉化与还原黑盒用例）全部通过。

---

## 2. 需求到用例映射矩阵

| 需求模块 | 规格要点 | 覆盖用例 |
|---|---|---|
| 1. 后端现场试跑引擎 (`tako_switch_lib::mcp_server`) | 公开类型 `LiveDemoRequest` / `LiveDemoCitation` / `LiveDemoResult`（`#[serde(rename_all = "camelCase")]`）与 `execute_live_demo_for_paths` | TC-LIVE-01 ~ TC-LIVE-09 |
| 1.1 未登录拦截 | 未配置 API Key 返回中文 `Err` | TC-LIVE-01 |
| 1.2 开关解耦 | `state.mcp.<tool> == false` 时仍允许现场试跑 | TC-LIVE-02 |
| 1.3 `audio` 试跑 | `POST /v1/audio/speech`（默认 `mimo-v2.5-tts` + `冰糖` + `mp3`），落盘并输出 `data:audio/mpeg;base64,...` | TC-LIVE-03 |
| 1.4 `image` 试跑 | `POST /v1/images/generations`，校验真实图片魔数落盘并输出 `data:image/...;base64,...` | TC-LIVE-04 |
| 1.5 `video` 试跑与断点恢复 | 三步异步 `POST -> GET -> GET /content`、校验 `ftyp`、输出 `data:video/mp4;base64,...`；`resume_task_id` 绝不触发 `POST` 二次扣费 | TC-LIVE-05, TC-LIVE-06 |
| 1.6 `search` 试跑 | `POST /v1/search`，解析 `citations` 与每个 URL 的 `domain` | TC-LIVE-07 |
| 1.7 `consult` 试跑 | `POST /v1/chat/completions`，返回第二模型正文 `text_output` | TC-LIVE-08 |
| 2. 前端内联剧场与去生硬化 | 移除 `McpShowcaseModal.tsx`，启用 `mcp-inline-theater`，禁止 JSX 纯文本裸露内部英文函数名，`data-mcp-tool` 与中文名齐全 | TC-V2-UI-01 ~ TC-V2-UI-03 |
| 3. 真媒体二进制校验 | 7 张 `.webp`（RIFF+WEBP）、`floating-garden-hq.mp4`（>100KB、`ftyp`、`avc1`、无 `hvc1`）、3 段冰糖 `.mp3`（>20KB、ID3/MPEG Sync）、移除 `voice-sample.wav` | TC-V2-MEDIA-01 ~ TC-V2-MEDIA-04 |
| 4. CSP 与安全红线 | `img-src` 与 `media-src` 均允许 `'self'` 和 `data:`，无 `*` / `'unsafe-eval'`；零 `app.asar` 篡改 | TC-V2-CSP-01, TC-V2-SEC-01 |

---

## 3. 后端 Tauri 现场试跑引擎黑盒测试用例表（`src-tauri/tests/mcp_live_demo_blackbox.rs`）

| 用例编号 | 前置条件 | 触发步骤 | 可判定预期（精确断言） | 边界条件 |
|---|---|---|---|---|
| **TC-LIVE-01** | `AppPaths::for_test` 沙盒；未调用 `save_switch_auth`（`auth.json` 不存在）；`state.base_url` 指向本地 Mock Server | 调用 `execute_live_demo_for_paths(&paths, req)`（如 `tool = "audio"`） | 1) 返回 `Err(msg)`；2) `msg` 非空且含中文字符（包含非 ASCII 提示，禁止空错或纯英文栈）；3) Mock Server 收到请求数为 `0` | 覆盖 API Key 缺失且不发起外网请求 |
| **TC-LIVE-02** | 已写入测试 Key；`state.mcp` 五个开关（`image`/`video`/`audio`/`search`/`consult`）全部设为 `false` 并落盘；Mock Server 就绪 | 分别以 `tool = "audio"` / `"consult"` 调用 `execute_live_demo_for_paths(&paths, req)` | 1) 均返回 `Ok(res)`，不报 `"当前已在 Tako Switch 中关闭"` 类错误；2) Mock Server 正常收到对应端点请求 | 验证现场试跑独立于客户端 MCP 注入总开关 |
| **TC-LIVE-03** | 已写入测试 Key；Mock Server 的 `POST /v1/audio/speech` 返回合法 MP3 字节（含 ID3/MPEG Sync 头） | 构造 `LiveDemoRequest`：`tool = "audio"`, `prompt = "你好，欢迎使用冰糖配音"`, 省略 `model` 与 `voice` | 1) Mock 收到 `POST /v1/audio/speech`，含 `Authorization: Bearer <key>`；2) 请求 JSON 含 `model == "mimo-v2.5-tts"`、`voice == "冰糖"`、`response_format == "mp3"`；3) `res.saved_path` 存在且位于沙盒目录内，字节与 Mock 响应完全一致；4) `res.preview_data_url` 以 `"data:audio/mpeg;base64,"` 开头且 Base64 解码与落盘字节一致；5) `res.model == "mimo-v2.5-tts"` | 覆盖默认音色/格式推断；可补充显式 `voice` + `instructions` 透传 |
| **TC-LIVE-04** | 已写入测试 Key；Mock Server 的 `POST /v1/images/generations` 分别配置：(A) 返回有效 PNG（`b64_json`）或 WebP；(B) 返回伪装图片体 `<html>bad</html>` | 分别以 `tool = "image"`, `prompt = "珊瑚色陶瓷马克杯"`, `aspect_ratio = Some("1:1")` 调用 `execute_live_demo_for_paths` | **(A) 正向**：1) 请求 `POST /v1/images/generations`；2) `res.saved_path` 落盘文件通过 PNG/JPEG/WebP 魔数校验且字节一致；3) `res.preview_data_url` 以 `"data:image/png;base64,"`（或对应 MIME）开头且解码一致。<br>**(B) 边界**：拒绝落盘并返回含中文说明的 `Err` | 覆盖非图片魔数响应拦截与零残余脏文件 |
| **TC-LIVE-05** | 已写入测试 Key；Mock Server 依次响应：`POST /v1/videos/generations` -> `{"id":"vid_live_001","status":"queued"}`；第 1 次 `GET /v1/videos/vid_live_001` -> `IN_PROGRESS`；第 2 次 `GET` -> `SUCCESS`；`GET /v1/videos/vid_live_001/content` -> 合法 `ftyp` MP4 字节 | 调用 `execute_live_demo_for_paths`（`tool = "video"`, `prompt = "云雾花园慢速推镜头"`） | 1) 请求严格按 `POST /v1/videos/generations` -> `GET /v1/videos/vid_live_001` (x2) -> `GET /v1/videos/vid_live_001/content` 顺序发生；2) `res.task_id == Some("vid_live_001")`；3) `res.saved_path` 落盘且含 `ftyp` 头，字节一致；4) `res.preview_data_url` 以 `"data:video/mp4;base64,"` 开头且解码一致；5) 边界：若 `/content` 返回非 `ftyp` 字节，返回 `Err` 且不落盘 | 覆盖异步状态机轮询与 MP4 容器魔数门禁 |
| **TC-LIVE-06** | 已写入测试 Key；Mock Server 记录所有请求路径与方法，对 `GET /v1/videos/vid_resume_777` 返回 `SUCCESS`，对 `/content` 返回合法 MP4 | 传入 `tool = "video"`, `resume_task_id = Some("vid_resume_777")` 调用 `execute_live_demo_for_paths` | 1) **关键防重扣断言**：`POST /v1/videos/generations` 请求次数严格等于 `0`；2) `GET /v1/videos/vid_resume_777` 与 `GET /v1/videos/vid_resume_777/content` 各至少请求 1 次；3) `res.task_id == Some("vid_resume_777")`、`res.saved_path` 落盘且 `preview_data_url` 为 `data:video/mp4;base64,...` | 防止视频超时/中断后恢复触发二次付费提交 |
| **TC-LIVE-07** | 已写入测试 Key；Mock Server `POST /v1/search` 返回多条含不同路径/端口 URL 的结果（如 `https://v2.tauri.app/security/csp/`、`https://developer.mozilla.org:443/docs`） | 调用 `execute_live_demo_for_paths`（`tool = "search"`, `prompt = "Tauri v2 CSP"`, `provider = Some("grok")`） | 1) Mock 收到 `POST /v1/search`，请求体含 `query` 与 `provider == "grok"`；2) `res.citations` 非空且保留全部条目；3) 每条 `LiveDemoCitation` 的 `title`、`url`、`snippet` 完整，且 `domain` 正确提取为 `"v2.tauri.app"` / `"developer.mozilla.org"` | 覆盖带路径及端口的域名解析 |
| **TC-LIVE-08** | 已写入测试 Key；Mock Server `POST /v1/chat/completions` 返回 `choices[0].message.content` | 调用 `execute_live_demo_for_paths`（`tool = "consult"`, `prompt = "检查异步任务并发盲点"`, `model = Some("gpt-6.2-sol")`） | 1) Mock 收到 `POST /v1/chat/completions`，请求体 `model == "gpt-6.2-sol"` 且 `messages` 含提问文本；2) `res.model == "gpt-6.2-sol"`；3) `res.text_output` 非空且包含第二模型返回的审查正文 | 覆盖未传 `model` 时回落 `cfg.consult_model` |
| **TC-LIVE-09** | `serde_json` 序列化/反序列化上下文 | 构造 camelCase JSON（含 `aspectRatio`、`resumeTaskId`）反序列化为 `LiveDemoRequest`；将 `LiveDemoResult` 序列化为 JSON | 1) `LiveDemoRequest` 成功接收 camelCase 字段；2) `LiveDemoResult` 与 `LiveDemoCitation` 输出包含 `elapsedMs`、`savedPath`、`previewDataUrl`、`textOutput`、`taskId`、`citations`（`title`/`url`/`snippet`/`domain`）且不含 snake_case 键名 | 保障 Tauri IPC 前后端字段名契约 |
| **TC-LIVE-10** | 基线已有的 12 条中文汉化测试 | 运行 `cargo test --test chinese_localization_blackbox` | 12/12 全部 `ok`，0 回归 | 回归保护 |

---

## 4. 前端内联剧场、真媒体二进制、CSP 与安全测试用例表（`scripts/verify_client_mcp_ui.py`）

| 用例编号 | 检查项 | 触发与判定条件 | 预期结果 |
|---|---|---|---|
| **TC-V2-UI-01** | 淘汰死板弹窗，启用内联剧场 | 检查 `src/components/McpShowcaseModal.tsx` 文件存在性及 `src/` 内引用；检查 `data-testid="mcp-inline-theater"` | 1) `src/components/McpShowcaseModal.tsx` **不存在**且源码无 `McpShowcaseModal` 导入/残留引用；2) 存在 `data-testid="mcp-inline-theater"` |
| **TC-V2-UI-02** | 去生硬化文案与 5 工具绑定 | AST/词法级扫描 `src/` 下 `.tsx`/`.jsx` 标签纯文本节点与 JSX 字符串子节点；检查 5 组 `(data-mcp-tool, 中文名)` | 1) JSX 标签可见文本节点中 **0 处**直接渲染 `tako_generate_image` / `tako_generate_video` / `tako_generate_audio` / `tako_web_search` / `tako_consult` / `tako_account_status`（仅允许在 `data-mcp-tool` 属性或常量映射中出现）；2) 5 个工具均存在 `data-mcp-tool="tako_..."` 与对应中文名（`生图`/`生视频`/`声音`/`联网`/`外脑`） |
| **TC-V2-UI-03** | 17 项 V2 关键交互 `data-testid` | 全量扫描 `src/`（兼容字面量与 React 模板字面量映射） | 以下 17 个 `data-testid` 全部命中：`open-mcp-demo`、`mcp-inline-theater`、`mcp-demo-tab-{image,video,audio,search,consult}`、`mcp-mode-showcase`、`mcp-mode-live`、`mcp-image-slider`、`mcp-demo-video`、`mcp-video-compare-frame`、`mcp-demo-play-audio`、`mcp-audio-subtitles`、`mcp-demo-copy-prompt`、`consult-preset-gpt-6.2-sol`、`mcp-live-run-btn` |
| **TC-V2-MEDIA-01** | 7 张真实 WebP 样本校验 | 读取 `public/mcp-demo/{ceramic-mug,ceramic-mug-edited,floating-garden,forest-cabin,multi-reference-1,multi-reference-2,multi-reference-comparison}.webp` | 全部存在；满足 `RIFF` + `WEBP` 魔数、RIFF 声明长度不越界、首块为 `VP8 `/`VP8L`/`VP8X`；7 个文件字节互不相同且在 `src/` 中被引用 |
| **TC-V2-MEDIA-02** | 真视频 MP4 与跨平台编码校验 | 读取 `public/mcp-demo/floating-garden-hq.mp4` | 1) 文件存在且体积 `> 100_000` 字节；2) 首 Box 为合法 `ftyp` 且容器 Box 结构完整；3) 包含 H.264 跨平台标识 `avc1`；4) **不含**苹果私有裸 `hvc1` 陷阱；5) 在 `src/` 中被引用 |
| **TC-V2-MEDIA-03** | 3 段真实「冰糖」MP3 校验 | 读取 `public/mcp-demo/voice-bingtang-{warm,lively,radio}.mp3` | 1) 3 个文件均存在且各自体积 `> 20_000` 字节；2) 均通过 `ID3`（含合法 synchsafe 标签长度与后续 MPEG Frame Sync）或 MPEG Frame Sync 字节头校验；3) 3 段音频字节互异且均在 `src/` 中被引用 |
| **TC-V2-MEDIA-04** | 旧机械合成音彻底清理 | 检查 `public/mcp-demo/voice-sample.wav` 存在性及 `src/` 引用 | `voice-sample.wav` 已从磁盘删除，且 `src/` 中无残留引用 |
| **TC-V2-CSP-01** | CSP 放行媒体与内联预览 | 解析 `src-tauri/tauri.conf.json` 中 `app.security.csp` | `img-src` 与 `media-src` 均显式包含 `'self'` 与 `data:`；`default-src`、`script-src`、`img-src`、`media-src` 均不含通配符 `*` 或 `'unsafe-eval'` |
| **TC-V2-SEC-01** | 反篡改红线 | 扫描 `src-tauri/src/`、`src/`、`scripts/` | 0 处匹配 `app.asar` / `@electron/asar` / 解包重打包调用 |
| **TC-V2-E2E-01** | 内联剧场与现场试跑端到端验收 | 在桌面端或 `npm run dev` 打开主界面 -> 点击 `open-mcp-demo` 展开 `mcp-inline-theater` -> 切换 5 个 Tab 与 `mcp-mode-showcase`/`mcp-mode-live` -> 拖动 `mcp-image-slider`、播放 `mcp-demo-video` 与 3 段冰糖 MP3、点击 `mcp-live-run-btn` | 无模态遮罩阻挡主页面；图片对比滑杆流畅；MP4 视频正常解码首帧与播放；3 段 MP3 与 `mcp-audio-subtitles` 同步切换且无 CSP 控制台报错；现场试跑返回真实结果或未登录中文提示 |

---

## 5. 待澄清边界与接口契约假设

1. **`LiveDemoRequest` / `LiveDemoResult` 字段可空性**：规格给出了结构体字段清单，但未逐一标注 `String` 与 `Option<String>`。黑盒集成测试 `mcp_live_demo_blackbox.rs` 通过 `serde_json::from_value::<LiveDemoRequest>` 构造输入（与真实 Tauri IPC 反序列化路径一致）并通过 `serde_json::to_value(&result)` 校验输出，从而同时验证 `#[serde(rename_all = "camelCase")]` 契约并兼容字段可选性设计。
2. **`LiveDemoCitation.domain` 对端口与 `www.` 的处理**：对于 `https://v2.tauri.app/security/csp/` 期望 `domain == "v2.tauri.app"`；若 URL 带显式端口（如 `:443`）或 `www.` 前缀，当前用例默认提取纯主机名（host without port）。
3. **音频/图片输出文件名与目录**：测试断言 `saved_path` 位于沙盒 `paths.default_media_output_dir()`（即 `<temp>/Downloads/Tako`）或 `state.mcp.output_dir` 之下且文件可读、内容与服务端一致，不硬编码具体时间戳文件名格式。
4. **`consult` 的 `text_output` 格式**：测试断言 `text_output` 包含第二模型返回的 `choices[0].message.content` 正文（允许外层带或不带 `[model]` 前缀）。
