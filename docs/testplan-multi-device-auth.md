# Tako Switch 多机 OAuth 设备 Key 不互踢黑盒测试计划

## 1. 测试范围与隔离原则

- **测试职责**：依据「多机同时使用 OAuth 设备 Key 不互踢」需求与公开接口契约，独立编写黑盒测试计划与 Rust 集成测试；严禁读取开发分支实现代码。
- **沙盒与网关隔离**：每个模拟设备使用独立的 `AppPaths::for_test(temp_dir)` 沙盒（`is_test = true`），互不共享配置或凭据；使用 `tokio::net::TcpListener` 搭建本地零外部依赖 Mock Tako 网关，1:1 复刻服务端 `tako-src` 按 `(user_id, client_id@device_name)` 签发与删旧 Key 的行为，不发起真实外网请求。
- **回归保护**：不破坏现有 `chinese_localization_blackbox.rs`、`mcp_live_demo_blackbox.rs` 与 `verify_client_mcp_ui.py`。

---

## 2. 公开契约与 Mock 服务端对齐说明

| 模块 | 公开符号 | 契约要求 |
|---|---|---|
| `tako_switch_lib::config` | `AppPaths::device_id_path(&self) -> PathBuf` | 返回 `<switch_dir>/device-id`（沙盒内持久化路径） |
| `tako_switch_lib::config` | `read_or_create_device_id(paths: &AppPaths) -> String` | 首次读取生成并持久化 8 位设备 ID；重复调用幂等返回同一 ID |
| `tako_switch_lib::config` | `sanitize_host_label(raw: &str) -> Option<String>` | 清洗主机名，仅保留合法 ASCII 标识字符；空白/纯无效字符返回 `None` |
| `tako_switch_lib::config` | `build_desktop_device_name_with_host(paths: &AppPaths, raw_host: Option<&str>) -> String` | 拼接主机标签与 8 位 `device-id`，长度 `1..=32`，字符集 `[A-Za-z0-9._@+-]` |
| `tako_switch_lib::config` | `build_desktop_device_name(paths: &AppPaths) -> String` | 读取宿主机主机名并委托生成合法 `device_name` |
| `tako_switch_lib::auth` | `DESKTOP_OAUTH_CLIENT_ID` | 常量值等于 `"takoswitch"`（不再使用 `"tako-connect"`） |
| `tako_switch_lib::auth` | `build_oauth_authorize_url(base_url, redirect_uri, oauth_state, code_challenge, device_name) -> String` | 生成 `/app/authorize?...` 授权 URL，含 `client_id=takoswitch`、`response_type=code`、`code_challenge_method=S256` 及编码后的 `device_name` |
| `tako_switch_lib::auth` | `exchange_desktop_token(paths, base_url, code, code_verifier, redirect_uri)` | 请求 `POST /api/desktop/token`（`client_id == "takoswitch"` + PKCE S256），写入 `auth.json`（`auth_source = "oauth"`） |
| `tako_switch_lib::auth` | `logout(paths)` | 若为 `oauth` 来源则请求 `POST /api/desktop/revoke` 吊销当前单把 Key，清空本地 `auth.json` 但保留 `device_id_path()` |
| `tako_switch_lib::auth` | `fetch_account_status(paths)` | 请求 `GET /api/desktop/me`（`Bearer <key>`），被删/吊销 Key 返回 `401` 且回退校验失败，`error.is_some()` |

### Mock Tako 网关行为复刻（`tako-src` 等价逻辑）
1. `GET /app/authorize`：校验 `client_id == "takoswitch"`、`response_type == "code"`、`code_challenge_method == "S256"`，将 `(user_id, client_id, redirect_uri, code_challenge, device_name)` 绑定到一次性 `code`，返回 `302` 重定向至 `redirect_uri?code=...&state=...`。
2. `POST /api/desktop/token`：校验 `grant_type == "authorization_code"`、`client_id == "takoswitch"`、一次性 `code`、PKCE `S256(code_verifier) == code_challenge`；拼出 `token_name = format!("{client_id}@{device_name}")`，**删除该用户下所有同名 `token_name` 的旧 Key**，再签发新 Key 返回 `{"success": true, "data": {"key": ..., "tokenName": ...}}`。
3. `GET /api/desktop/me`：校验 `Authorization: Bearer <key>` 是否仍在有效表中；失效或已被同名替换/吊销则返回 `401`。
4. `POST /api/desktop/revoke`：按 `Bearer <key>` 精确删除单把 Key，不影响同用户其他设备的 Key。
5. `POST /apiStats/api/verify-identity`、`POST /apiStats/api/get-key-id`、`POST /apiStats/api/user-quota`：与有效 Key 表联动，已吊销 Key 不再返回有效身份或 `apiId`。

---

## 3. 黑盒测试用例矩阵（`src-tauri/tests/multi_device_oauth_blackbox.rs`）

| 用例编号 | 用例名称 | 前置条件 | 触发步骤 | 精确可判定断言 | 边界条件 |
|---|---|---|---|---|---|
| **TC-MD-01** | 同系统同名双机并发不互踢 | 两个独立沙盒 `paths_a`、`paths_b` 均配置指向本地 Mock 网关 | 两机均传入相同主机名 `Some("MacBook-Pro.local")`，经 `build_oauth_authorize_url` -> `/app/authorize` -> `exchange_desktop_token` 完成登录；随后并发执行 `tokio::join!(fetch_account_status(&paths_a), fetch_account_status(&paths_b))` | 1) 两机 `read_or_create_device_id` 均为 8 位且互异；2) 两机 `device_name` 与服务端 `token_name` 均不同且各自包含本机 8 位 `device-id`；3) 服务端有效 Key 数为 `2`；4) 并发 `fetch_account_status` 均满足 `logged_in == true && error.is_none()`；5) 两机本地保存的 Key 互异 | 模拟同一用户两台同型号 MacBook 同名并行在线 |
| **TC-MD-02** | 同机重登幂等替换不泄漏令牌 | `paths_a` 与 `paths_b` 已完成 TC-MD-01 登录（当前服务端 2 把有效 Key） | `paths_a` 再次传入 `Some("MacBook-Pro.local")` 执行完整 OAuth 登录流程 | 1) `paths_a` 的 `device-id` 与重登前完全一致；2) `paths_a` 本地拿到新 Key（与旧 Key 不同）；3) 服务端总有效令牌数仍为 `2`（旧 Key 已被精准替换删除，无垃圾堆积）；4) 用旧 Key 请求 `/api/desktop/me` 返回 `401`；5) `paths_a` 与 `paths_b` 调用 `fetch_account_status` 均 `logged_in == true && error.is_none()` | 覆盖重复授权场景的幂等替换与防泄漏 |
| **TC-MD-03** | 单机登出精确吊销且保留本机 `device-id` | `paths_a` 与 `paths_b` 均处于 OAuth 已登录状态 | 对 `paths_a` 执行 `logout(&paths_a)` | 1) `logout` 返回 `logged_in == false`，`read_main_api_key(&paths_a).is_none()`；2) `paths_a.device_id_path()` 依然存在且读回原 8 位 `device-id`；3) 服务端收到 `/api/desktop/revoke` 且仅吊销 `paths_a` 的 Key，有效 Key 剩余 `1` 把（为 `paths_b`）；4) `paths_b` 调用 `fetch_account_status(&paths_b)` 仍为 `logged_in == true && error.is_none()` | 验证登出不破坏本地设备身份且不误伤他机 |
| **TC-MD-04** | 极端主机名清洗与 32 字符硬上限 | 独立沙盒 `paths` | 分别传入 80 字符超长主机名、`"Shiroha 的 MacBook Pro (16-inch).local"`、`""`、`"   "`、`"..."`、`"---"`、`"中文主机名"`、`None` 调用 `build_desktop_device_name_with_host` 与 `sanitize_host_label`；并调用 `build_desktop_device_name(&paths)` | 1) 所有生成结果的字节长度严格满足 `1 <= len <= 32`；2) 所有字符均属于 `[A-Za-z0-9._@+-]`；3) 结果均包含本机 8 位 `device-id`；4) `sanitize_host_label` 对 `""` / `"   "` / 纯中文返回 `None`，对含非 ASCII 的混合主机名返回合法清洗结果 | 覆盖中文、空格、括号、纯符号、空串、超长主机名边界 |
| **TC-MD-05** | 授权 URL 契约校验 | 无需网络连接 | 检查 `DESKTOP_OAUTH_CLIENT_ID`，并用普通 `device_name` 及含 `+`/`@` 特殊字符的 `device_name` 调用 `build_oauth_authorize_url` | 1) `DESKTOP_OAUTH_CLIENT_ID == "takoswitch"`；2) URL 以 `{base_url}/app/authorize?` 开头；3) 参数包含 `client_id=takoswitch`、`response_type=code`、`code_challenge_method=S256` 及 percent-encode 后的 `device_name`（如 `+` -> `%2B`、`@` -> `%40`）；4) 全文**不含**旧版 `"tako-connect"` | 校验 OAuth 参数与 URL 编码安全 |
| **TC-MD-06** | 拒绝旧 `client_id` 与错误 PKCE 校验 | Mock 网关运行中 | 校验直接向 `/api/desktop/token` 发送旧 `client_id="tako-connect"` 或错误 `code_verifier` 时均被拒绝；未登录沙盒调用 `fetch_account_status` | 1) 服务端返回 `400` 且未签发新 Key；2) 未登录沙盒返回 `logged_in == false` | 防止回归回退到旧 client_id 或跳过 PKCE |
