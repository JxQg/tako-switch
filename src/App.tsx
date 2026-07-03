import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  CheckCircle2,
  Eye,
  KeyRound,
  Loader2,
  RefreshCw,
  RotateCcw,
  Save,
  Settings2,
  ShieldCheck,
  Terminal
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

type ConfigInput = {
  gatewayBaseUrl: string;
  apiKey: string;
  codexModel?: string;
  claudeModel?: string;
  configureCodex: boolean;
  configureClaude: boolean;
};

type ToolStatus = {
  name: string;
  installed: boolean;
  version?: string;
  error?: string;
};

type ExistingConfig = {
  target: string;
  path: string;
  exists: boolean;
  content: string;
};

type LoadedConfigs = {
  codex: ExistingConfig;
  claude: ExistingConfig;
};

type FilePreview = {
  target: string;
  path: string;
  exists: boolean;
  backupPath: string;
  before: string;
  after: string;
};

type EnvPreview = {
  name: string;
  maskedValue: string;
  note: string;
};

type PreviewResult = {
  files: FilePreview[];
  envUpdates: EnvPreview[];
  warnings: string[];
};

type AppliedFile = {
  target: string;
  path: string;
  backupPath: string;
  created: boolean;
};

type ApplyResult = {
  files: AppliedFile[];
  envUpdates: string[];
  tools: ToolStatus[];
  warnings: string[];
};

type RestoreResult = {
  target: string;
  path: string;
  restoredFrom: string;
  deletedTarget: boolean;
};

const emptyPreview: PreviewResult = {
  files: [],
  envUpdates: [],
  warnings: []
};

function App() {
  const [form, setForm] = useState<ConfigInput>({
    gatewayBaseUrl: "http://127.0.0.1:3000/v1",
    apiKey: "",
    codexModel: "gpt-5.4",
    claudeModel: "",
    configureCodex: true,
    configureClaude: true
  });
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [configs, setConfigs] = useState<LoadedConfigs | null>(null);
  const [preview, setPreview] = useState<PreviewResult>(emptyPreview);
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [restoreResult, setRestoreResult] = useState<RestoreResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"loading" | "preview" | "apply" | "restore" | null>(
    null
  );

  const validation = useMemo(() => validateLocal(form), [form]);
  const canSubmit = validation.length === 0 && busy === null;

  useEffect(() => {
    void refreshState();
  }, []);

  async function refreshState() {
    setBusy("loading");
    setError(null);
    try {
      const [toolStatuses, loadedConfigs] = await Promise.all([
        invoke<ToolStatus[]>("detect_tools"),
        invoke<LoadedConfigs>("load_current_configs")
      ]);
      setTools(toolStatuses);
      setConfigs(loadedConfigs);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function createPreview(event?: FormEvent) {
    event?.preventDefault();
    if (!canSubmit && busy !== null) return;
    setBusy("preview");
    setError(null);
    setResult(null);
    setRestoreResult(null);
    try {
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: form });
      setPreview(nextPreview);
    } catch (err) {
      setError(String(err));
      setPreview(emptyPreview);
    } finally {
      setBusy(null);
    }
  }

  async function applyConfigs() {
    if (!canSubmit) return;
    setBusy("apply");
    setError(null);
    setRestoreResult(null);
    try {
      const applyResult = await invoke<ApplyResult>("apply_configs", { input: form });
      setResult(applyResult);
      setTools(applyResult.tools);
      await refreshConfigsOnly();
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: form });
      setPreview(nextPreview);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function refreshConfigsOnly() {
    try {
      setConfigs(await invoke<LoadedConfigs>("load_current_configs"));
    } catch (err) {
      setError(String(err));
    }
  }

  async function restore(file: AppliedFile) {
    setBusy("restore");
    setError(null);
    setRestoreResult(null);
    try {
      const restored = await invoke<RestoreResult>("restore_backup", {
        target: file.target,
        backupPath: file.backupPath
      });
      setRestoreResult(restored);
      await refreshConfigsOnly();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  const loading = busy !== null;

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">Tako Switch</p>
          <h1>Codex / Claude Code 一键配置</h1>
        </div>
        <button className="icon-button" onClick={refreshState} disabled={loading} title="重新检测">
          {busy === "loading" ? <Loader2 className="spin" /> : <RefreshCw />}
        </button>
      </header>

      <section className="status-strip" aria-label="工具检测状态">
        {tools.length === 0 ? (
          <StatusItem label="检测中" detail="正在读取本机状态" installed={false} />
        ) : (
          tools.map((tool) => (
            <StatusItem
              key={tool.name}
              label={tool.name}
              detail={tool.version || tool.error || "未检测到命令"}
              installed={tool.installed}
            />
          ))
        )}
      </section>

      {error && (
        <div className="notice error" role="alert">
          <AlertTriangle />
          <span>{error}</span>
        </div>
      )}

      <div className="workspace">
        <form className="panel form-panel" onSubmit={createPreview}>
          <div className="panel-heading">
            <Settings2 />
            <div>
              <h2>基础配置</h2>
              <p>填写网关、密钥和需要写入的客户端。</p>
            </div>
          </div>

          <fieldset className="target-grid">
            <label className={form.configureCodex ? "target active" : "target"}>
              <input
                type="checkbox"
                checked={form.configureCodex}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    configureCodex: event.target.checked
                  }))
                }
              />
              <Terminal />
              <span>Codex</span>
            </label>
            <label className={form.configureClaude ? "target active" : "target"}>
              <input
                type="checkbox"
                checked={form.configureClaude}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    configureClaude: event.target.checked
                  }))
                }
              />
              <Terminal />
              <span>Claude Code</span>
            </label>
          </fieldset>

          <label className="field">
            <span>LLM 网关地址</span>
            <input
              value={form.gatewayBaseUrl}
              placeholder="http://127.0.0.1:3000/v1"
              onChange={(event) =>
                setForm((current) => ({
                  ...current,
                  gatewayBaseUrl: event.target.value
                }))
              }
            />
          </label>

          <label className="field">
            <span>API Key / Token</span>
            <div className="secret-input">
              <KeyRound />
              <input
                type="password"
                value={form.apiKey}
                placeholder="粘贴网关密钥"
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    apiKey: event.target.value
                  }))
                }
              />
            </div>
          </label>

          <div className="field-grid">
            <label className="field">
              <span>Codex 模型</span>
              <input
                value={form.codexModel || ""}
                disabled={!form.configureCodex}
                placeholder="gpt-5.4"
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    codexModel: event.target.value
                  }))
                }
              />
            </label>
            <label className="field">
              <span>Claude 模型</span>
              <input
                value={form.claudeModel || ""}
                disabled={!form.configureClaude}
                placeholder="留空则使用 Claude Code 默认模型"
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    claudeModel: event.target.value
                  }))
                }
              />
            </label>
          </div>

          {validation.length > 0 && (
            <div className="notice soft">
              <AlertTriangle />
              <span>{validation[0]}</span>
            </div>
          )}

          <div className="button-row">
            <button className="secondary" type="submit" disabled={!canSubmit}>
              {busy === "preview" ? <Loader2 className="spin" /> : <Eye />}
              <span>生成预览</span>
            </button>
            <button className="primary" type="button" disabled={!canSubmit} onClick={applyConfigs}>
              {busy === "apply" ? <Loader2 className="spin" /> : <Save />}
              <span>应用配置</span>
            </button>
          </div>
        </form>

        <section className="panel">
          <div className="panel-heading">
            <ShieldCheck />
            <div>
              <h2>写入预览</h2>
              <p>密钥已遮罩，应用前会自动生成备份。</p>
            </div>
          </div>

          {preview.envUpdates.length > 0 && (
            <div className="env-list">
              {preview.envUpdates.map((item) => (
                <div className="env-row" key={item.name}>
                  <span>{item.name}</span>
                  <code>{item.maskedValue}</code>
                  <small>{item.note}</small>
                </div>
              ))}
            </div>
          )}

          {preview.warnings.map((warning) => (
            <div className="notice soft" key={warning}>
              <AlertTriangle />
              <span>{warning}</span>
            </div>
          ))}

          {preview.files.length === 0 ? (
            <EmptyState text="点击“生成预览”查看将写入的配置。" />
          ) : (
            <div className="preview-stack">
              {preview.files.map((file) => (
                <PreviewBlock key={file.target} file={file} />
              ))}
            </div>
          )}
        </section>
      </div>

      <section className="panel results-panel">
        <div className="panel-heading">
          <CheckCircle2 />
          <div>
            <h2>结果与恢复</h2>
            <p>成功后可从这里查看写入路径，并恢复最近一次备份。</p>
          </div>
        </div>

        {!result && !restoreResult ? (
          <EmptyState text="还没有写入结果。" />
        ) : (
          <div className="result-grid">
            {result?.files.map((file) => (
              <div className="result-row" key={`${file.target}-${file.path}`}>
                <div>
                  <strong>{file.target === "codex" ? "Codex" : "Claude Code"}</strong>
                  <span>{file.created ? "已创建配置" : "已更新配置"}</span>
                  <code>{file.path}</code>
                  <small>备份：{file.backupPath}</small>
                </div>
                <button className="secondary compact" onClick={() => restore(file)} disabled={loading}>
                  {busy === "restore" ? <Loader2 className="spin" /> : <RotateCcw />}
                  <span>恢复</span>
                </button>
              </div>
            ))}

            {result?.envUpdates.map((item) => (
              <div className="result-row" key={item}>
                <div>
                  <strong>环境变量</strong>
                  <span>{item}</span>
                </div>
              </div>
            ))}

            {restoreResult && (
              <div className="notice success">
                <CheckCircle2 />
                <span>
                  已恢复 {restoreResult.target}：{restoreResult.deletedTarget ? "目标文件已删除" : restoreResult.path}
                </span>
              </div>
            )}
          </div>
        )}
      </section>

      {configs && (
        <section className="panel current-panel">
          <div className="panel-heading">
            <Terminal />
            <div>
              <h2>当前配置</h2>
              <p>只读视图，用于确认现有文件位置。</p>
            </div>
          </div>
          <div className="current-grid">
            <CurrentConfigBlock config={configs.codex} />
            <CurrentConfigBlock config={configs.claude} />
          </div>
        </section>
      )}
    </main>
  );
}

function StatusItem({
  label,
  detail,
  installed
}: {
  label: string;
  detail: string;
  installed: boolean;
}) {
  return (
    <div className={installed ? "status-item installed" : "status-item"}>
      {installed ? <CheckCircle2 /> : <AlertTriangle />}
      <div>
        <strong>{label}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}

function PreviewBlock({ file }: { file: FilePreview }) {
  return (
    <article className="preview-block">
      <div className="file-meta">
        <strong>{file.target === "codex" ? "Codex config.toml" : "Claude settings.json"}</strong>
        <span>{file.exists ? "更新已有文件" : "创建新文件"}</span>
        <code>{file.path}</code>
        <small>备份将写入：{file.backupPath}</small>
      </div>
      <div className="diff-grid">
        <label>
          <span>当前</span>
          <textarea readOnly value={file.before || "(文件不存在或为空)"} />
        </label>
        <label>
          <span>写入后</span>
          <textarea readOnly value={file.after} />
        </label>
      </div>
    </article>
  );
}

function CurrentConfigBlock({ config }: { config: ExistingConfig }) {
  return (
    <article className="current-block">
      <strong>{config.target === "codex" ? "Codex" : "Claude Code"}</strong>
      <span>{config.exists ? "已存在" : "未创建"}</span>
      <code>{config.path}</code>
      <textarea readOnly value={config.content || "(文件不存在或为空)"} />
    </article>
  );
}

function EmptyState({ text }: { text: string }) {
  return <div className="empty-state">{text}</div>;
}

function validateLocal(form: ConfigInput) {
  const errors: string[] = [];
  if (!form.configureCodex && !form.configureClaude) {
    errors.push("至少选择 Codex 或 Claude Code。");
  }
  if (!/^https?:\/\/.+/i.test(form.gatewayBaseUrl.trim())) {
    errors.push("网关地址必须以 http:// 或 https:// 开头。");
  }
  if (!form.apiKey.trim()) {
    errors.push("API Key / Token 不能为空。");
  }
  if (form.configureCodex && !form.codexModel?.trim()) {
    errors.push("选择 Codex 时必须填写 Codex 模型。");
  }
  return errors;
}

export default App;
