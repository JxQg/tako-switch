import { useEffect, useRef, useState } from "react";
import {
  Brain,
  Check,
  Copy,
  ExternalLink,
  Film,
  Globe,
  Headphones,
  Image as ImageIcon,
  Pause,
  Play,
  ShieldCheck,
  Sparkles,
  Volume2,
  X,
} from "lucide-react";
import { api, type McpToolsConfig } from "../lib/api";

export type McpToolKey = "image" | "video" | "audio" | "search" | "consult";

interface McpShowcaseModalProps {
  open: boolean;
  initialTab: McpToolKey;
  mcp: McpToolsConfig | null;
  savingMcp: boolean;
  onClose: () => void;
  onToggleMcp: (key: McpToolKey) => void;
  onSelectConsultModel: (model: string) => void;
}

const DEMO_CONSULT_PRESETS = [
  "gpt-6.2-sol",
  "gpt-6.2-astra",
  "claude-opus-5-5",
  "deepseek-v4.1-flash",
  "swe-2",
] as const;

interface TabMeta {
  key: McpToolKey;
  label: string;
  toolName: string;
  summary: string;
  modelBadge: string;
  prompt: string;
  Icon: typeof ImageIcon;
}

const TAB_METAS: TabMeta[] = [
  {
    key: "image",
    label: "生图",
    toolName: "tako_generate_image",
    summary: "支持文生图、自然语言改图与多参考图风格融合，成品自动保存本地目录。",
    modelBadge: "gpt-image-2 / grok-imagine-image",
    prompt:
      "帮我用 Tako 生图生成一张珊瑚色陶瓷马克杯的产品静物图，再在此基础上改成森林绿釉面并保留自然光影。",
    Icon: ImageIcon,
  },
  {
    key: "video",
    label: "生视频",
    toolName: "tako_generate_video",
    summary: "支持首尾帧锁定插值、多图角色一致、已有 MP4 自然语言编辑与尾帧续写。",
    modelBadge: "grok-imagine-video-1.5",
    prompt:
      "用 Tako 生视频把这张珊瑚色马克杯作为首帧、森林绿釉面杯作为尾帧，生成 5 秒慢速推进镜头并平滑渐变釉色。",
    Icon: Film,
  },
  {
    key: "audio",
    label: "声音",
    toolName: "tako_generate_audio",
    summary: "默认「冰糖」高表现力配音，支持一句话控制情绪语气与本地音频声音克隆。",
    modelBadge: "mimo-v2.5-tts · 冰糖音色",
    prompt:
      "用 Tako 声音以温柔轻快的「冰糖」音色朗读这段产品介绍开场白，在重点卖点处稍微放慢语速。",
    Icon: Headphones,
  },
  {
    key: "search",
    label: "联网",
    toolName: "tako_web_search",
    summary: "实时联网检索最新技术文档与新闻，返回带真实 URL 的可溯源引用。",
    modelBadge: "实时信源 · results[].url 溯源",
    prompt:
      "用 Tako 联网检索 Tauri v2 中 security.csp 的 media-src 最新配置规范，并附上官方文档引用链接。",
    Icon: Globe,
  },
  {
    key: "consult",
    label: "外脑",
    toolName: "tako_consult",
    summary: "主 Agent 遇到架构盲点或卡壳时，随时呼叫第二模型做独立交叉会诊。",
    modelBadge: "跨模型交叉审查",
    prompt:
      "用 Tako 外脑帮我审查当前异步任务超时恢复方案的并发竞态与重复扣费盲点，给出 3 条可落地改进建议。",
    Icon: Brain,
  },
];

const VIDEO_CAPABILITIES = [
  {
    id: "keyframes",
    label: "首尾帧插值",
    detail: "锁定首帧与尾帧画面构图，自动补全中间镜头推进与光影色彩过渡。",
  },
  {
    id: "reference",
    label: "最多14张 @image0 角色一致",
    detail: "通过 @image0 ~ @image13 锚定人物、商品外观与场景道具，跨镜头不走样。",
  },
  {
    id: "mp4-edit",
    label: "已有 MP4 自然语言编辑",
    detail: "直接输入本地 MP4 视频路径，一句话替换背景天气、色调或局部元素。",
  },
  {
    id: "extend",
    label: "2-10s 尾帧续写",
    detail: "基于既有视频最后一帧继续延展 2–10 秒后续动作，无缝拼接长片段。",
  },
] as const;

const SEARCH_CITATIONS = [
  {
    domain: "v2.tauri.app",
    title: "Content Security Policy (CSP) | Tauri v2 Security",
    snippet:
      "Configure app.security.csp in tauri.conf.json. Add media-src 'self' data:; to allow bundled local audio and video assets.",
    url: "https://v2.tauri.app/security/csp/",
  },
  {
    domain: "developer.mozilla.org",
    title: "Content-Security-Policy: media-src directive - MDN",
    snippet:
      "Restricts the URLs which can be loaded using <audio>, <video>, and <track> elements; falls back to default-src if omitted.",
    url: "https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/media-src",
  },
] as const;

const WAVEFORM_BARS = [
  28, 46, 72, 54, 88, 64, 42, 78, 95, 68, 48, 84, 92, 58, 40, 74, 86, 52, 66, 44,
  70, 50, 36, 60,
];

function formatAudioTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

export function McpShowcaseModal({
  open,
  initialTab,
  mcp,
  savingMcp,
  onClose,
  onToggleMcp,
  onSelectConsultModel,
}: McpShowcaseModalProps) {
  const [activeTab, setActiveTab] = useState<McpToolKey>(initialTab);
  const [selectedVideoCap, setSelectedVideoCap] = useState<string>("keyframes");
  const [copiedTab, setCopiedTab] = useState<McpToolKey | null>(null);

  const audioRef = useRef<HTMLAudioElement | null>(null);
  const copyTimerRef = useRef<number | null>(null);
  const [audioPlaying, setAudioPlaying] = useState(false);
  const [audioCurrentTime, setAudioCurrentTime] = useState(0);
  const [audioDuration, setAudioDuration] = useState(0);
  const [audioError, setAudioError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setActiveTab(initialTab);
      setCopiedTab(null);
      setAudioError(null);
    } else {
      setAudioPlaying(false);
      setAudioCurrentTime(0);
    }
  }, [open, initialTab]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  useEffect(() => {
    if (activeTab !== "audio" && audioRef.current) {
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
      setAudioPlaying(false);
      setAudioCurrentTime(0);
    }
  }, [activeTab]);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  if (!open) return null;

  const currentMeta = TAB_METAS.find((t) => t.key === activeTab) ?? TAB_METAS[0];
  const isToolEnabled = Boolean(mcp?.[activeTab]);
  const outputDir = mcp?.outputDir?.trim() || "~/Downloads/Tako";
  const consultModel = mcp?.consultModel?.trim() || "gpt-6.2-sol";
  const audioProgressPct =
    audioDuration > 0 ? Math.min(100, (audioCurrentTime / audioDuration) * 100) : 0;

  const handleTogglePlayAudio = async () => {
    const el = audioRef.current;
    if (!el) return;
    setAudioError(null);
    if (audioPlaying) {
      el.pause();
      setAudioPlaying(false);
      return;
    }
    try {
      await el.play();
      setAudioPlaying(true);
    } catch (err) {
      setAudioPlaying(false);
      setAudioError(`音频播放受限：${String(err)}`);
    }
  };

  const handleSeekAudio = (e: React.MouseEvent<HTMLDivElement>) => {
    const el = audioRef.current;
    if (!el || !audioDuration) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    el.currentTime = ratio * audioDuration;
    setAudioCurrentTime(el.currentTime);
  };

  const handleCopyPrompt = async () => {
    const text = currentMeta.prompt;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const ta = document.createElement("textarea");
        ta.value = text;
        ta.style.position = "fixed";
        ta.style.opacity = "0";
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
      }
      setCopiedTab(activeTab);
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = window.setTimeout(() => {
        setCopiedTab(null);
      }, 2000);
    } catch {
      setCopiedTab(activeTab);
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = window.setTimeout(() => {
        setCopiedTab(null);
      }, 2000);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-3 sm:p-4"
      onClick={onClose}
    >
      <style>{`
        @keyframes mcp-video-scan {
          0% { transform: translateX(-100%); }
          100% { transform: translateX(260%); }
        }
        @keyframes mcp-wave-pulse {
          0%, 100% { transform: scaleY(0.55); opacity: 0.72; }
          50% { transform: scaleY(1.12); opacity: 1; }
        }
        .mcp-video-scan-bar {
          animation: mcp-video-scan 2.2s ease-in-out infinite;
        }
        .mcp-wave-active {
          transform-origin: bottom center;
          animation: mcp-wave-pulse 0.85s ease-in-out infinite;
        }
        @media (prefers-reduced-motion: reduce) {
          .mcp-video-scan-bar,
          .mcp-wave-active {
            animation: none;
          }
        }
      `}</style>

      <div
        role="dialog"
        aria-modal="true"
        aria-label="五大超级 MCP 能力演示"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-[580px] max-h-[88vh] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] flex flex-col overflow-hidden"
      >
        {/* Header */}
        <div className="px-4 sm:px-5 py-3 border-b border-[var(--border)] flex items-center justify-between gap-2 shrink-0">
          <div className="flex items-center gap-2 min-w-0 flex-wrap">
            <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0" />
            <h3 className="text-[14px] font-semibold text-[var(--text-primary)] truncate">
              五大超级 MCP 能力演示
            </h3>
            <span className="px-2 py-0.5 rounded-full text-[11px] font-medium bg-[var(--success-soft)] border border-[var(--success-border)] text-[var(--success)] shrink-0">
              零额度消耗 · 本地实拍样本
            </span>
          </div>

          <button
            type="button"
            onClick={onClose}
            aria-label="关闭演示弹窗"
            className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer shrink-0"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 5-Tab Segmented Switch Bar */}
        <div className="px-4 sm:px-5 pt-3 pb-2 border-b border-[var(--border)] bg-[var(--bg-canvas)]/55 shrink-0">
          <div className="grid grid-cols-5 gap-1 p-1 rounded-[11px] bg-[var(--bg-pill)] border border-[var(--border)]">
            {TAB_METAS.map(({ key, label, Icon }) => {
              const active = activeTab === key;
              const enabled = Boolean(mcp?.[key]);
              return (
                <button
                  key={key}
                  type="button"
                  data-testid={`mcp-demo-tab-${key}`}
                  onClick={() => setActiveTab(key)}
                  className={`h-8 px-1.5 rounded-[8px] text-[12px] flex items-center justify-center gap-1 transition cursor-pointer min-w-0 ${
                    active
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-semibold shadow-[var(--shadow-segment)] border border-[var(--border)]"
                      : "border border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  <Icon
                    className={`h-3.5 w-3.5 shrink-0 ${
                      active ? "text-[var(--accent)]" : ""
                    }`}
                  />
                  <span className="truncate">{label}</span>
                  <span
                    className={`h-1.5 w-1.5 rounded-full shrink-0 hidden sm:inline-block ${
                      enabled ? "bg-[var(--success)]" : "bg-[var(--border-strong)]"
                    }`}
                  />
                </button>
              );
            })}
          </div>
        </div>

        {/* Scrollable Tab Content */}
        <div className="flex-1 min-h-0 overflow-y-auto subtle-scroll p-4 sm:p-5 flex flex-col gap-3.5">
          {/* Block 1: Capability Overview Strip */}
          <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] px-3.5 py-2.5 flex flex-wrap sm:flex-nowrap items-center justify-between gap-2.5">
            <div className="min-w-0 flex-1 flex flex-col gap-1">
              <div className="flex items-center gap-1.5 flex-wrap">
                <code className="text-[11px] font-mono px-1.5 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] border border-[var(--accent-border)]">
                  {currentMeta.toolName}
                </code>
                <span className="text-[11px] px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-secondary)]">
                  {activeTab === "image" || activeTab === "video" || activeTab === "audio"
                    ? `默认存至 ${outputDir}`
                    : currentMeta.modelBadge}
                </span>
              </div>
              <p className="text-[12px] text-[var(--text-secondary)] leading-snug">
                {currentMeta.summary}
              </p>
            </div>

            <div className="flex items-center gap-2 shrink-0">
              <span
                className={`text-[11px] font-medium ${
                  isToolEnabled ? "text-[var(--success)]" : "text-[var(--text-muted)]"
                }`}
              >
                {isToolEnabled ? "已开启" : "已关闭"}
              </span>
              <button
                type="button"
                role="switch"
                aria-checked={isToolEnabled}
                aria-label={`切换${currentMeta.label} MCP 开关`}
                disabled={!mcp || savingMcp}
                onClick={() => onToggleMcp(activeTab)}
                className={`relative inline-flex h-[20px] w-[36px] shrink-0 items-center rounded-full transition-colors duration-150 cursor-pointer disabled:opacity-50 ${
                  isToolEnabled ? "bg-[var(--accent)]" : "bg-[var(--border-strong)]"
                }`}
              >
                <span
                  className={`inline-block h-[16px] w-[16px] rounded-full bg-white shadow-sm transition-transform duration-150 ${
                    isToolEnabled ? "translate-x-[18px]" : "translate-x-[2px]"
                  }`}
                />
              </button>
            </div>
          </div>

          {/* Block 2: Visual Real-Sample Demo Area */}
          {activeTab === "image" && (
            <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3.5 flex flex-col gap-3">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
                  <div className="aspect-square bg-[var(--bg-subtle)] overflow-hidden">
                    <img
                      src="/mcp-demo/ceramic-mug.webp"
                      alt="1. 文生图原图 · 珊瑚色陶瓷马克杯"
                      className="w-full h-full object-cover"
                    />
                  </div>
                  <div className="px-2.5 py-2 border-t border-[var(--border)] text-[11.5px] font-medium text-[var(--text-primary)]">
                    1. 文生图原图 · 珊瑚色陶瓷马克杯
                  </div>
                </div>

                <div className="rounded-[10px] border border-[var(--accent-border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
                  <div className="aspect-square bg-[var(--bg-subtle)] overflow-hidden">
                    <img
                      src="/mcp-demo/ceramic-mug-edited.webp"
                      alt="2. 一句话改图 · 换成森林绿釉面并保留光影"
                      className="w-full h-full object-cover"
                    />
                  </div>
                  <div className="px-2.5 py-2 border-t border-[var(--border)] text-[11.5px] font-medium text-[var(--accent)]">
                    2. 一句话改图 · 换成森林绿釉面并保留光影
                  </div>
                </div>
              </div>

              <div className="rounded-[9px] bg-[var(--bg-subtle)]/75 border border-[var(--border)] px-3 py-2 text-[11.5px] text-[var(--text-secondary)]">
                支持{" "}
                <code className="font-mono text-[var(--text-primary)]">gpt-image-2</code> /{" "}
                <code className="font-mono text-[var(--text-primary)]">
                  gpt-image-2.5-flare
                </code>{" "}
                /{" "}
                <code className="font-mono text-[var(--text-primary)]">
                  grok-imagine-image
                </code>{" "}
                /{" "}
                <code className="font-mono text-[var(--text-primary)]">
                  gemini-3.1-flash-image
                </code>{" "}
                及最多 <span className="font-semibold text-[var(--text-primary)]">3 张参考图融合</span>
              </div>
            </div>
          )}

          {activeTab === "video" && (
            <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3.5 flex flex-col gap-3">
              {/* Keyframe Interpolation Visual Card */}
              <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] p-3 flex flex-col gap-2.5">
                <div className="flex items-center justify-between gap-2 flex-wrap">
                  <span className="text-[12px] font-semibold text-[var(--text-primary)]">
                    首尾帧锁定插值 & 异步进度防重扣
                  </span>
                  <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] border border-[var(--accent-border)]">
                    grok-imagine-video-1.5
                  </span>
                </div>

                <div className="flex flex-col sm:flex-row items-center gap-2.5">
                  <div className="w-full sm:w-[116px] shrink-0 rounded-[9px] border border-[var(--border)] bg-[var(--bg-canvas)] overflow-hidden">
                    <div className="h-[78px] bg-[var(--bg-subtle)] overflow-hidden">
                      <img
                        src="/mcp-demo/ceramic-mug.webp"
                        alt="首帧 ceramic-mug.webp"
                        className="w-full h-full object-cover"
                      />
                    </div>
                    <div className="px-2 py-1 text-[10.5px] text-[var(--text-secondary)] truncate text-center">
                      首帧 · ceramic-mug.webp
                    </div>
                  </div>

                  <div className="flex-1 w-full min-w-0 flex flex-col items-center gap-1.5 px-1">
                    <div className="text-[11px] font-medium text-[var(--accent)] text-center leading-tight">
                      ──[镜头推进与釉色渐变插值动效]──▶
                    </div>
                    <div className="w-full h-2 rounded-full bg-[var(--bg-subtle)] border border-[var(--border)] overflow-hidden relative">
                      <div
                        className="h-full w-2/5 rounded-full mcp-video-scan-bar"
                        style={{
                          background:
                            "linear-gradient(90deg, transparent, var(--accent), transparent)",
                        }}
                      />
                    </div>
                    <div className="text-[10.5px] text-[var(--text-muted)] tabular-nums">
                      00:00 ── 插值生成 5.0s MP4 ── 00:05
                    </div>
                  </div>

                  <div className="w-full sm:w-[116px] shrink-0 rounded-[9px] border border-[var(--accent-border)] bg-[var(--bg-canvas)] overflow-hidden">
                    <div className="h-[78px] bg-[var(--bg-subtle)] overflow-hidden">
                      <img
                        src="/mcp-demo/ceramic-mug-edited.webp"
                        alt="尾帧 ceramic-mug-edited.webp"
                        className="w-full h-full object-cover"
                      />
                    </div>
                    <div className="px-2 py-1 text-[10.5px] text-[var(--accent)] truncate text-center font-medium">
                      尾帧 · ceramic-mug-edited.webp
                    </div>
                  </div>
                </div>
              </div>

              {/* 4 Switchable Capability Chips */}
              <div className="flex flex-wrap gap-1.5">
                {VIDEO_CAPABILITIES.map((cap) => {
                  const active = selectedVideoCap === cap.id;
                  return (
                    <button
                      key={cap.id}
                      type="button"
                      onClick={() => setSelectedVideoCap(cap.id)}
                      className={`px-2.5 py-1 rounded-full text-[11px] font-medium border transition cursor-pointer ${
                        active
                          ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)]"
                          : "bg-[var(--bg-card)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      {cap.label}
                    </button>
                  );
                })}
              </div>

              <div className="rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] px-3 py-2 text-[11.5px] text-[var(--text-secondary)]">
                {VIDEO_CAPABILITIES.find((c) => c.id === selectedVideoCap)?.detail}
              </div>

              <div className="rounded-[9px] border border-[var(--success-border)] bg-[var(--success-soft)] px-3 py-2 flex items-center gap-2 text-[11.5px] font-medium text-[var(--success)]">
                <ShieldCheck className="h-4 w-4 shrink-0" />
                <span>
                  超时返回 <code className="font-mono font-semibold">task_id</code> 免重复扣费恢复
                </span>
              </div>
            </div>
          )}

          {activeTab === "audio" && (
            <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3.5 flex flex-col gap-3">
              <audio
                ref={audioRef}
                src="/mcp-demo/voice-sample.wav"
                preload="metadata"
                onLoadedMetadata={(e) => setAudioDuration(e.currentTarget.duration || 0)}
                onTimeUpdate={(e) => setAudioCurrentTime(e.currentTarget.currentTime || 0)}
                onEnded={() => {
                  setAudioPlaying(false);
                  setAudioCurrentTime(0);
                }}
                onError={() => {
                  setAudioPlaying(false);
                  setAudioError("音频样本加载失败");
                }}
              />

              <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] p-3.5 flex flex-col gap-3">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="h-7 w-7 rounded-lg bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[var(--accent)] flex items-center justify-center shrink-0">
                      <Volume2 className="h-3.5 w-3.5" />
                    </span>
                    <div className="min-w-0">
                      <div className="text-[12.5px] font-semibold text-[var(--text-primary)] truncate">
                        「冰糖」标准配音试听样本
                      </div>
                      <div className="text-[11px] text-[var(--text-muted)] font-mono truncate">
                        /mcp-demo/voice-sample.wav
                      </div>
                    </div>
                  </div>
                  <span className="tabular-nums text-[11px] font-mono text-[var(--text-secondary)] shrink-0">
                    {formatAudioTime(audioCurrentTime)} / {formatAudioTime(audioDuration)}
                  </span>
                </div>

                {/* Controls + Dynamic Waveform */}
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    data-testid="mcp-demo-play-audio"
                    onClick={() => void handleTogglePlayAudio()}
                    aria-label={audioPlaying ? "暂停试听" : "播放试听"}
                    className="h-9 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium inline-flex items-center gap-1.5 shrink-0 transition cursor-pointer"
                  >
                    {audioPlaying ? (
                      <>
                        <Pause className="h-3.5 w-3.5" />
                        <span>暂停</span>
                      </>
                    ) : (
                      <>
                        <Play className="h-3.5 w-3.5" />
                        <span>试听</span>
                      </>
                    )}
                  </button>

                  <div
                    className="flex-1 min-w-0 flex items-end gap-[3px] h-8 px-2 py-1 rounded-[8px] bg-[var(--bg-canvas)] border border-[var(--border)] cursor-pointer"
                    onClick={handleSeekAudio}
                    title="点击波形进度跳转"
                  >
                    {WAVEFORM_BARS.map((heightPct, idx) => {
                      const barPct = ((idx + 1) / WAVEFORM_BARS.length) * 100;
                      const played = audioProgressPct >= barPct;
                      return (
                        <span
                          key={idx}
                          className={`flex-1 rounded-full transition-colors ${
                            audioPlaying ? "mcp-wave-active" : ""
                          }`}
                          style={{
                            height: `${Math.max(22, heightPct)}%`,
                            backgroundColor:
                              played || audioPlaying ? "var(--accent)" : "var(--border-strong)",
                            animationDelay: audioPlaying ? `${(idx % 6) * 90}ms` : undefined,
                          }}
                        />
                      );
                    })}
                  </div>
                </div>

                {/* Progress Bar */}
                <div
                  className="h-1.5 w-full rounded-full bg-[var(--bg-subtle)] overflow-hidden cursor-pointer"
                  onClick={handleSeekAudio}
                >
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-all duration-100"
                    style={{ width: `${audioProgressPct}%` }}
                  />
                </div>

                {audioError && (
                  <div className="text-[11px] text-[var(--danger)]">{audioError}</div>
                )}
              </div>

              <div className="rounded-[9px] bg-[var(--bg-subtle)]/75 border border-[var(--border)] px-3 py-2 text-[11.5px] text-[var(--text-secondary)] leading-relaxed">
                本地试听样本 · 实际调用默认采用{" "}
                <code className="font-mono text-[var(--text-primary)]">mimo-v2.5-tts</code>{" "}
                「冰糖」音色，支持一句话调语气与{" "}
                <span className="font-semibold text-[var(--text-primary)]">
                  10MB 内本地音频声音克隆
                </span>
              </div>
            </div>
          )}

          {activeTab === "search" && (
            <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3.5 flex flex-col gap-3">
              <div className="rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] px-3 py-2 flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  <Globe className="h-3.5 w-3.5 text-[var(--accent)] shrink-0" />
                  <span className="text-[12px] font-medium text-[var(--text-primary)] truncate">
                    Tauri v2 CSP media-src 配置与最新变更
                  </span>
                </div>
                <span className="text-[10.5px] font-mono px-1.5 py-0.5 rounded bg-[var(--success-soft)] text-[var(--success)] shrink-0">
                  2 条可溯源引用
                </span>
              </div>

              <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] p-3 text-[12px] text-[var(--text-secondary)] leading-relaxed">
                <div className="text-[11px] font-semibold text-[var(--text-muted)] mb-1">
                  实时信源提炼摘要
                </div>
                Tauri v2 的 <code className="font-mono text-[var(--text-primary)]">security.csp</code>{" "}
                若配置了 <code className="font-mono text-[var(--text-primary)]">default-src 'self'</code>{" "}
                而未单独声明 <code className="font-mono text-[var(--text-primary)]">media-src</code>，
                内置音视频资源与 Data URI 将被回退规则拦截；补充{" "}
                <code className="font-mono text-[var(--text-primary)]">media-src 'self' data:;</code>{" "}
                即可安全放行本地 <code className="font-mono text-[var(--text-primary)]">&lt;audio&gt;</code>{" "}
                播放。
              </div>

              <div className="flex flex-col gap-2">
                {SEARCH_CITATIONS.map((item) => (
                  <button
                    key={item.url}
                    type="button"
                    onClick={() => void api.openUrl(item.url).catch(() => {})}
                    className="text-left rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] hover:border-[var(--accent-border)] p-2.5 flex flex-col gap-1 transition cursor-pointer"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[10.5px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--accent)] font-medium">
                        {item.domain}
                      </span>
                      <ExternalLink className="h-3 w-3 text-[var(--text-muted)] shrink-0" />
                    </div>
                    <div className="text-[12px] font-semibold text-[var(--text-primary)]">
                      {item.title}
                    </div>
                    <div className="text-[11px] text-[var(--text-secondary)] line-clamp-2">
                      {item.snippet}
                    </div>
                    <div className="text-[10.5px] font-mono text-[var(--text-muted)] truncate">
                      {item.url}
                    </div>
                  </button>
                ))}
              </div>

              <div className="rounded-[9px] border border-[var(--accent-border)] bg-[var(--accent-soft)] px-3 py-1.5 text-[11.5px] font-medium text-[var(--accent)]">
                真实引用 <code className="font-mono font-semibold">results[].url</code>，拒绝离线幻觉瞎编链接
              </div>
            </div>
          )}

          {activeTab === "consult" && (
            <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-canvas)] p-3.5 flex flex-col gap-3">
              {/* Dual-Model Cross-Consultation Stream */}
              <div className="flex flex-col gap-2.5">
                <div className="self-start max-w-[92%] rounded-[11px] border border-[var(--border)] bg-[var(--bg-card)] px-3 py-2 flex flex-col gap-1">
                  <span className="text-[10.5px] font-semibold text-[var(--text-muted)]">
                    主 Agent · 遇到并发架构盲点提问
                  </span>
                  <span className="text-[12px] text-[var(--text-primary)] leading-snug">
                    「网关异步生视频 90 秒超时后，客户端自动重试容易造成重复扣费，当前状态机还有哪些并发盲点？」
                  </span>
                </div>

                <div className="self-end max-w-[94%] rounded-[11px] border border-[var(--accent-border)] bg-[var(--accent-soft)] px-3 py-2.5 flex flex-col gap-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[10.5px] font-semibold text-[var(--accent)]">
                      外脑参谋 · {consultModel}
                    </span>
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--bg-card)] text-[var(--text-secondary)] border border-[var(--border)]">
                      3 条独立盲点审查
                    </span>
                  </div>
                  <ol className="text-[11.5px] text-[var(--text-primary)] flex flex-col gap-1 list-decimal list-inside leading-snug">
                    <li>
                      请求前生成稳定 <code className="font-mono">idempotency_key</code>，网关层按 key 幂等去重
                    </li>
                    <li>
                      状态机隔离 <code className="font-mono">pending / running / settled</code>，禁止对运行中任务二次计费
                    </li>
                    <li>
                      超时立即下发 <code className="font-mono">task_id</code>，后续切只读轮询接口恢复结果
                    </li>
                  </ol>
                </div>
              </div>

              {/* Quick Consult Model Preset Switcher */}
              <div className="rounded-[10px] border border-[var(--border)] bg-[var(--bg-card)] p-2.5 flex flex-col gap-2">
                <div className="flex items-center justify-between gap-2 flex-wrap">
                  <span className="text-[11.5px] font-medium text-[var(--text-secondary)]">
                    默认外脑参谋模型快捷切换
                  </span>
                  <span className="text-[11px] font-mono text-[var(--accent)]">
                    当前: {consultModel}
                  </span>
                </div>
                <div className="flex items-center gap-1.5 flex-wrap">
                  {DEMO_CONSULT_PRESETS.map((m) => {
                    const active = consultModel === m;
                    return (
                      <button
                        key={m}
                        type="button"
                        data-testid={`consult-preset-${m}`}
                        disabled={!mcp || savingMcp}
                        onClick={() => onSelectConsultModel(m)}
                        className={`px-2 py-1 rounded-md text-[11px] font-mono border transition cursor-pointer disabled:opacity-60 ${
                          active
                            ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-semibold"
                            : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]"
                        }`}
                      >
                        {m}
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Block 3: Bottom Copy-Prompt Action Bar */}
        <div className="px-4 sm:px-5 py-3 border-t border-[var(--border)] bg-[var(--bg-card)] flex flex-col sm:flex-row sm:items-center justify-between gap-2.5 shrink-0">
          <div className="min-w-0 flex-1 flex flex-col gap-0.5">
            <span className="text-[10.5px] font-medium text-[var(--text-muted)]">
              实战指令 · 复制后可直接粘贴到 Codex / Claude / 反重力触发
            </span>
            <p className="text-[12px] text-[var(--text-primary)] selectable truncate sm:whitespace-normal">
              {currentMeta.prompt}
            </p>
          </div>

          <button
            type="button"
            data-testid="mcp-demo-copy-prompt"
            onClick={() => void handleCopyPrompt()}
            className="h-8 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium inline-flex items-center justify-center gap-1.5 shrink-0 transition cursor-pointer"
          >
            {copiedTab === activeTab ? (
              <>
                <Check className="h-3.5 w-3.5" />
                <span>已复制指令</span>
              </>
            ) : (
              <>
                <Copy className="h-3.5 w-3.5" />
                <span>复制实战指令</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
