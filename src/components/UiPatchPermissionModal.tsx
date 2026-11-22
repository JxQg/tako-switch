import { CheckCircle2, ExternalLink, RefreshCw, ShieldAlert, X } from "lucide-react";
import type { UiPatchPermissionPrompt } from "../lib/api";

interface UiPatchPermissionModalProps {
  prompts: UiPatchPermissionPrompt[];
  retrying: boolean;
  onOpenMacosSettings: () => void;
  onRetry: () => void;
  onDismiss: () => void;
}

function kindHeading(prompt: UiPatchPermissionPrompt): string {
  switch (prompt.kind) {
    case "macos_app_management":
      return "需要 macOS「App 管理」权限";
    case "windows_uac":
      return "需要 Windows 管理员权限（UAC）";
    case "windows_file_lock":
      return `${prompt.appName} 正在运行并占用资源包`;
    case "windows_msix_read_only":
      return "Microsoft Store (MSIX) 目录受系统只读保护";
  }
}

export function UiPatchPermissionModal({
  prompts,
  retrying,
  onOpenMacosSettings,
  onRetry,
  onDismiss,
}: UiPatchPermissionModalProps) {
  if (prompts.length === 0) return null;

  const hasMacosAppMgmt = prompts.some((p) => p.kind === "macos_app_management");
  const allMsixReadOnly = prompts.every((p) => p.kind === "windows_msix_read_only");

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 backdrop-blur-[2px] p-4"
      onClick={onDismiss}
    >
      <div
        data-testid="ui-patch-permission-modal"
        className="w-full max-w-[480px] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-2xl overflow-hidden flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-5 py-3.5 border-b border-[var(--border)] flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <ShieldAlert className="h-4 w-4 text-[var(--warning)] shrink-0" />
            <h2 className="text-[14px] font-semibold text-[var(--text-primary)] truncate">
              界面汉化需要系统写入授权
            </h2>
          </div>
          <button
            type="button"
            aria-label="关闭权限引导弹窗"
            onClick={onDismiss}
            className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="px-5 py-4 flex flex-col gap-3 max-h-[68vh] overflow-y-auto">
          <div className="rounded-[10px] border border-[var(--success-border)] bg-[var(--success-soft)] px-3 py-2.5 flex items-start gap-2 text-[12px] text-[var(--success)]">
            <CheckCircle2 className="h-4 w-4 shrink-0 mt-0.5" />
            <span>
              配置文件级简体中文与中文回复规则已正常生效；仅桌面端界面资源包写入受到系统权限保护拦截。
            </span>
          </div>

          {prompts.map((prompt, idx) => (
            <div
              key={`${prompt.appId}-${prompt.kind}-${idx}`}
              className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-subtle)]/55 p-3.5 flex flex-col gap-2"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-[13px] font-semibold text-[var(--text-primary)]">
                  {prompt.appName} · {kindHeading(prompt)}
                </span>
                <span className="text-[11px] px-1.5 py-0.5 rounded bg-[var(--bg-card)] border border-[var(--border)] text-[var(--text-secondary)] shrink-0">
                  {prompt.action === "restore" ? "还原原版" : "界面汉化"}
                </span>
              </div>

              <p className="text-[12px] leading-relaxed text-[var(--text-secondary)]">
                {prompt.message}
              </p>

              {prompt.kind === "macos_app_management" && (
                <ol className="text-[12px] text-[var(--text-secondary)] space-y-1 pl-4 list-decimal">
                  <li>点击下方「打开系统设置」跳转至「隐私与安全性 → App 管理」</li>
                  <li>在列表中开启 <strong>Tako Switch</strong> 右侧的授权开关</li>
                  <li>返回本窗口点击「重试界面汉化」即可自动完成注入</li>
                </ol>
              )}

              {prompt.kind === "windows_uac" && (
                <ol className="text-[12px] text-[var(--text-secondary)] space-y-1 pl-4 list-decimal">
                  <li>点击下方「重试」并在 Windows UAC 弹窗中选择「是」授予管理员权限</li>
                  <li>仅在本次写入/还原安装包时临时提权，完成后自动退出提权进程</li>
                </ol>
              )}

              {prompt.kind === "windows_file_lock" && (
                <p className="text-[11.5px] text-[var(--text-muted)]">
                  请先完全退出 {prompt.appName}（含右下角托盘图标），然后点击「重试」。
                </p>
              )}

              {prompt.targetPath && (
                <div className="text-[11px] font-mono text-[var(--text-muted)] bg-[var(--bg-card)] border border-[var(--border)] rounded-md px-2.5 py-1.5 break-all">
                  {prompt.targetPath}
                </div>
              )}
            </div>
          ))}
        </div>

        <div className="px-5 py-3 border-t border-[var(--border)] flex flex-wrap items-center justify-end gap-2 bg-[var(--bg-card)]">
          {hasMacosAppMgmt && (
            <button
              type="button"
              data-testid="open-app-management-settings-btn"
              onClick={onOpenMacosSettings}
              className="h-8 px-3 rounded-[9px] border border-[var(--accent-border)] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] text-[12px] font-medium inline-flex items-center gap-1.5 cursor-pointer"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              <span>打开系统设置 → App 管理</span>
            </button>
          )}

          {!allMsixReadOnly && (
            <button
              type="button"
              data-testid="retry-ui-patch-btn"
              disabled={retrying}
              onClick={onRetry}
              className="h-8 px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium inline-flex items-center gap-1.5 cursor-pointer disabled:opacity-60"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${retrying ? "animate-spin" : ""}`} />
              <span>{retrying ? "重试中..." : "重试"}</span>
            </button>
          )}

          <button
            type="button"
            data-testid="dismiss-permission-modal-btn"
            onClick={onDismiss}
            className="h-8 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
          >
            稍后再说
          </button>
        </div>
      </div>
    </div>
  );
}
