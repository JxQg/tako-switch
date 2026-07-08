# Tako Switch 测试编写指南

本文档约定项目内测试的组织方式和编写原则。目标是让测试高类聚、低耦合：测试按业务模块归档，生产代码不为了测试扩大公开 API。

## 目录约定

- Rust 测试统一放在 `src-tauri/src/tests/`。
- 与生产模块一一对应的测试文件使用同名路径，例如 `src-tauri/src/platforms/codex.rs` 的测试放在 `src-tauri/src/tests/platforms/codex.rs`。
- 生产模块底部只保留很薄的测试入口：

```rust
#[cfg(test)]
#[path = "../tests/platforms/codex.rs"]
mod tests;
```

- 通用测试夹具放在 `src-tauri/src/tests/mod.rs`，例如默认 provider writer、临时目录生成器等。
- 未来如果新增前端单元测试，再在 `src/tests/` 或按测试框架约定建立目录；不要把前端测试塞进 Rust 的 `src-tauri/src/tests/`。

## 编写原则

- 测试文件按业务能力聚合，不按“工具函数集合”随意堆放。
- 优先测试稳定行为：配置合并结果、输入校验、备份路径、脱敏输出、deep-link 解析等。
- 不为了测试把私有函数改成 `pub`。需要覆盖私有逻辑时，使用当前的 `#[path = "..."] mod tests;` 方式让测试仍作为被测模块的子模块编译。
- 测试数据应尽量小而明确，避免依赖真实用户目录、真实网络、真实账号或真实命令行环境。
- 涉及环境变量、临时目录、全局路径覆盖的测试必须使用 `install_dir_test_lock()` 串行保护，并在测试结束时清理环境变量和临时文件。
- 共享夹具只放真正跨多个测试文件复用的内容。单个模块专用的构造函数留在对应测试文件中。

## 新增 Rust 测试步骤

1. 找到被测生产模块，例如 `src-tauri/src/providers/validation.rs`。
2. 在 `src-tauri/src/tests/` 下创建对应测试文件，例如 `src-tauri/src/tests/providers/validation.rs`。
3. 在生产模块底部添加路径测试入口。
4. 在测试文件顶部使用 `use super::*;` 访问被测模块的内部函数。
5. 如果需要共享夹具，从 `crate::tests` 引入，例如：

```rust
use crate::tests::default_platform_writer;
```

6. 运行：

```bash
rtk bun run rust:test
```

## 命名建议

- 测试函数名使用行为描述：`merge_preserves_existing_fields_and_is_idempotent`。
- 临时目录名带上业务场景：`unique_temp_dir("provider-invalid-test")`。
- 断言优先表达用户可感知或模块契约，不断言无关格式细节。

## 提交前检查

改动 Rust 逻辑或测试时至少运行：

```bash
rtk bun run rust:test
```

如果同时改动前端类型、构建配置或文档示例，继续运行：

```bash
rtk bun run typecheck
rtk bun run build
```
