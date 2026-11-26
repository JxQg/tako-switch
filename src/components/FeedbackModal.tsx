import { useEffect, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  Copy,
  ExternalLink,
  Loader2,
  MessageSquare,
  Send,
  ShieldCheck,
  X,
} from "lucide-react";
import {
  api,
  TAKO_FEEDBACK_URL,
  type FeedbackCategory,
  type FeedbackReceipt,
} from "../lib/api";
import { ToggleSwitch } from "./UiPrimitives";

interface FeedbackModalProps {
  open: boolean;
  onClose: () => void;
}

const CATEGORIES: { id: FeedbackCategory; label: string }[] = [
  { id: "bug", label: "Bug / 异常" },
  { id: "install", label: "安装 / 下载" },
  { id: "i18n", label: "界面汉化" },
  { id: "mcp", label: "MCP 能力" },
  { id: "feature", label: "功能建议" },
  { id: "other", label: "其他" },
];

export function FeedbackModal({ open, onClose }: FeedbackModalProps) {
  const [category, setCategory] = useState<FeedbackCategory>("bug");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [contact, setContact] = useState("");
  const [includeDiagnostics, setIncludeDiagnostics] = useState(true);

  const [showDiagPreview, setShowDiagPreview] = useState(false);
  const [diagPreview, setDiagPreview] = useState<string | null>(null);
  const [loadingDiag, setLoadingDiag] = useState(false);

  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<FeedbackReceipt | null>(null);
  const [copiedId, setCopiedId] = useState(false);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setCopiedId(false);
  }, [open]);

  useEffect(() => {
    if (!open || !showDiagPreview || diagPreview !== null) return;
    let active = true;
    setLoadingDiag(true);
    api
      .getSanitizedDiagnosticsPreview()
      .then((data) => {
        if (active) {
          setDiagPreview(JSON.stringify(data, null, 2));
        }
      })
      .catch((err) => {
        if (active) {
          setDiagPreview(`无法读取诊断信息预览: ${String(err)}`);
        }
      })
      .finally(() => {
        if (active) setLoadingDiag(false);
      });
    return () => {
      active = false;
    };
  }, [open, showDiagPreview, diagPreview]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, submitting, onClose]);

  if (!open) return null;

  const resetAndClose = () => {
    if (submitting) return;
    setReceipt(null);
    setError(null);
    setDiagPreview(null);
    setShowDiagPreview(false);
    onClose();
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!description.trim()) {
      setError("请填写具体问题描述后再提交");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const res = await api.submitUserFeedback({
        category,
        title: title.trim(),
        description: description.trim(),
        contact: contact.trim(),
        includeDiagnostics,
      });
      setReceipt(res);
      setTitle("");
      setDescription("");
    } catch (err) {
      setError(typeof err === "string" ? err : err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const handleCopyTicketId = () => {
    if (!receipt) return;
    void navigator.clipboard?.writeText(`#${receipt.id}`).then(() => {
      setCopiedId(true);
      window.setTimeout(() => setCopiedId(false), 1800);
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
      onClick={resetAndClose}
    >
      <div
        data-testid="feedback-modal"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-[520px] max-h-[86vh] flex flex-col rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] overflow-hidden"
      >
        {/* Header */}
        <div className="px-5 py-3.5 border-b border-[var(--border)] flex items-center justify-between gap-3 shrink-0">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="w-8 h-8 rounded-[9px] bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[var(--accent)] flex items-center justify-center shrink-0">
              <MessageSquare className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">
                问题反馈与建议
              </h3>
              <p className="text-[11px] text-[var(--text-muted)]">
                直达 Tako 运维团队，帮助我们更快定位与修复问题
              </p>
            </div>
          </div>
          <button
            type="button"
            aria-label="关闭反馈弹窗"
            disabled={submitting}
            onClick={resetAndClose}
            className="p-1.5 rounded-[8px] text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer disabled:opacity-40"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Body */}
        {receipt ? (
          <div className="px-6 py-8 flex flex-col items-center text-center gap-3.5">
            <div className="w-12 h-12 rounded-full bg-[var(--success-soft)] text-[var(--success)] flex items-center justify-center">
              <CheckCircle2 className="h-6 w-6" />
            </div>
            <div>
              <div className="text-[15px] font-semibold text-[var(--text-primary)]">
                感谢你的反馈，已收到！
              </div>
              <div className="text-[12.5px] text-[var(--text-secondary)] mt-1">
                你的反馈编号为{" "}
                <span className="font-mono font-semibold text-[var(--text-primary)]">
                  #{receipt.id}
                </span>
                ，我们会在数据中台尽快查看处理。
              </div>
            </div>
            <div className="flex items-center gap-2 pt-2">
              <button
                type="button"
                onClick={handleCopyTicketId}
                className="h-8.5 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] inline-flex items-center gap-1.5 cursor-pointer"
              >
                <Copy className="h-3.5 w-3.5" />
                <span>{copiedId ? "已复制编号" : `复制 #${receipt.id}`}</span>
              </button>
              <button
                type="button"
                onClick={() => setReceipt(null)}
                className="h-8.5 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
              >
                继续提交
              </button>
              <button
                type="button"
                onClick={resetAndClose}
                className="h-8.5 px-4 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold cursor-pointer"
              >
                完成
              </button>
            </div>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="flex flex-col min-h-0 flex-1">
            <div className="px-5 py-4 overflow-y-auto subtle-scroll flex flex-col gap-3.5">
              {/* Category Pills */}
              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-medium text-[var(--text-primary)]">
                  反馈类型
                </label>
                <div className="flex flex-wrap gap-1.5">
                  {CATEGORIES.map((item) => {
                    const active = category === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => setCategory(item.id)}
                        className={`h-7.5 px-2.5 rounded-[8px] text-[12px] font-medium border transition cursor-pointer ${
                          active
                            ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)]"
                            : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]"
                        }`}
                      >
                        {item.label}
                      </button>
                    );
                  })}
                </div>
              </div>

              {/* Optional Title */}
              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-medium text-[var(--text-primary)]">
                  简短标题 <span className="text-[11px] font-normal text-[var(--text-muted)]">（选填）</span>
                </label>
                <input
                  type="text"
                  maxLength={120}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="例如：Claude 汉化后启动异常 / 某页面漏翻"
                  className="h-8.5 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12.5px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                />
              </div>

              {/* Required Description */}
              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-medium text-[var(--text-primary)]">
                  问题描述 <span className="text-[var(--danger)]">*</span>
                </label>
                <textarea
                  rows={4}
                  maxLength={4000}
                  required
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="请描述你遇到的问题、复现步骤或期望的改进..."
                  className="px-3 py-2 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12.5px] leading-relaxed text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)] resize-y min-h-[96px]"
                />
              </div>

              {/* Optional Contact */}
              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-medium text-[var(--text-primary)]">
                  联系方式 <span className="text-[11px] font-normal text-[var(--text-muted)]">（选填，QQ / 微信 / 邮箱，方便跟进）</span>
                </label>
                <input
                  type="text"
                  maxLength={100}
                  value={contact}
                  onChange={(e) => setContact(e.target.value)}
                  placeholder="留空则为完全匿名提交"
                  className="h-8.5 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12.5px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                />
              </div>

              {/* Diagnostics Toggle + Expandable Preview */}
              <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3 flex flex-col gap-2">
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1.5">
                      <ShieldCheck className="h-3.5 w-3.5 text-[var(--success)] shrink-0" />
                      <span>附带脱敏诊断信息</span>
                    </div>
                    <div className="text-[11px] text-[var(--text-muted)] mt-0.5 leading-snug">
                      仅含版本号、系统架构、客户端安装/汉化/MCP 开关状态，不含任何 Key、Token 或个人路径。
                    </div>
                  </div>
                  <ToggleSwitch
                    checked={includeDiagnostics}
                    ariaLabel="附带脱敏诊断信息"
                    onChange={setIncludeDiagnostics}
                  />
                </div>

                {includeDiagnostics && (
                  <div className="pt-1 border-t border-[var(--border)]/70">
                    <button
                      type="button"
                      onClick={() => setShowDiagPreview((prev) => !prev)}
                      className="text-[11.5px] text-[var(--accent)] hover:underline inline-flex items-center gap-1 cursor-pointer"
                    >
                      <ChevronDown
                        className={`h-3.5 w-3.5 transition-transform ${
                          showDiagPreview ? "rotate-180" : ""
                        }`}
                      />
                      <span>{showDiagPreview ? "收起诊断信息预览" : "查看将要附带的脱敏诊断信息"}</span>
                    </button>
                    {showDiagPreview && (
                      <div className="mt-2 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] p-2.5 max-h-[150px] overflow-auto subtle-scroll">
                        {loadingDiag ? (
                          <div className="text-[11px] text-[var(--text-muted)] inline-flex items-center gap-1.5">
                            <Loader2 className="h-3 w-3 animate-spin" />
                            <span>正在生成脱敏快照...</span>
                          </div>
                        ) : (
                          <pre className="text-[10.5px] font-mono text-[var(--text-secondary)] whitespace-pre-wrap break-all leading-relaxed">
                            {diagPreview ?? "{}"}
                          </pre>
                        )}
                      </div>
                    )}
                  </div>
                )}
              </div>

              {error && (
                <div
                  className="rounded-[9px] border px-3 py-2 text-[12px]"
                  style={{
                    backgroundColor: "var(--danger-soft)",
                    borderColor: "rgba(201, 76, 76, 0.25)",
                    color: "var(--danger)",
                  }}
                >
                  {error}
                </div>
              )}
            </div>

            {/* Footer */}
            <div className="px-5 py-3 border-t border-[var(--border)] flex items-center justify-between gap-2 shrink-0 bg-[var(--bg-card)]">
              <button
                type="button"
                onClick={() => void api.openUrl(TAKO_FEEDBACK_URL)}
                className="inline-flex items-center gap-1.5 text-[11.5px] text-[var(--text-muted)] hover:text-[var(--text-primary)] cursor-pointer"
              >
                <ExternalLink className="h-3.5 w-3.5" />
                <span>或前往 GitHub Issues</span>
              </button>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  disabled={submitting}
                  onClick={resetAndClose}
                  className="h-8.5 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer disabled:opacity-45"
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={submitting || !description.trim()}
                  className="h-8.5 px-4 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold inline-flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
                >
                  {submitting ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      <span>提交中...</span>
                    </>
                  ) : (
                    <>
                      <Send className="h-3.5 w-3.5" />
                      <span>提交反馈</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
