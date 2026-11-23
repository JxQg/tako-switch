import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Brain,
  CheckCircle2,
  ChevronDown,
  ExternalLink,
  Eye,
  EyeOff,
  Film,
  Globe,
  Globe2,
  Headphones,
  Image as ImageIcon,
  RefreshCw,
  Sparkles,
  X,
} from "lucide-react";
import {
  api,
  type McpToolsConfig,
  type NodeRuntimeInfo,
} from "../lib/api";
import { McpInlineTheater, type McpToolKey } from "./McpInlineTheater";
import { MultiPlatformSyncPanel } from "./MultiPlatformSyncPanel";
import { SegmentedControl, StatusPill, ToggleSwitch } from "./UiPrimitives";

export type McpCenterSection = "builtin" | "mcp-sync" | "skill-sync";

interface McpCardSpec {
  key: McpToolKey;
  mcpToolAttr: string;
  title: string;
  description: string;
  Icon: typeof ImageIcon;
}

const CORE_MCP_CARDS: McpCardSpec[] = [
  {
    key: "image",
    mcpToolAttr: "tako_generate_image",
    title: "图像生成",
    description: "文字生图、局部改图与多图融合。",
    Icon: ImageIcon,
  },
  {
    key: "video",
    mcpToolAttr: "tako_generate_video",
    title: "视频生成",
    description: "首尾帧控制与高质感动态生视频成片。",
    Icon: Film,
  },
  {
    key: "audio",
    mcpToolAttr: "tako_generate_audio",
    title: "声音生成",
    description: "多角色、多情绪自然语音旁白。",
    Icon: Headphones,
  },
  {
    key: "search",
    mcpToolAttr: "tako_web_search",
    title: "联网搜索",
    description: "实时权威信源检索与引用溯源。",
    Icon: Globe,
  },
  {
    key: "consult",
    mcpToolAttr: "tako_consult",
    title: "多模型会诊",
    description: "邀请第二外脑模型独立审阅与交叉判断。",
    Icon: Brain,
  },
];

interface McpCenterPanelProps {
  mcp: McpToolsConfig | null;
  savingMcp: boolean;
  loggedIn: boolean;
  loggingIn: boolean;
  initialDrawerTab?: McpToolKey | null;
  initialSection?: McpCenterSection | null;
  onConsumeInitialTab?: () => void;
  onConsumeInitialSection?: () => void;
  onNotify?: (msg: string) => void;
  onGoHome: () => void;
  onToggleCoreTool: (key: McpToolKey) => void;
  onToggleWebPage: (next: boolean) => Promise<void>;
  onSaveEdgeOneConfig: (token: string, projectName: string) => Promise<void>;
  onSelectConsultModel: (model: string) => void;
  onRequestLogin: () => void;
}

export function McpCenterPanel({
  mcp,
  savingMcp,
  loggedIn,
  loggingIn,
  initialDrawerTab,
  initialSection,
  onConsumeInitialTab,
  onConsumeInitialSection,
  onNotify,
  onGoHome,
  onToggleCoreTool,
  onToggleWebPage,
  onSaveEdgeOneConfig,
  onSelectConsultModel,
  onRequestLogin,
}: McpCenterPanelProps) {
  const [activeSection, setActiveSection] = useState<McpCenterSection>("builtin");
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [drawerTab, setDrawerTab] = useState<McpToolKey>("image");

  useEffect(() => {
    if (initialDrawerTab) {
      setActiveSection("builtin");
      setDrawerTab(initialDrawerTab);
      setDrawerOpen(true);
      onConsumeInitialTab?.();
    }
  }, [initialDrawerTab, onConsumeInitialTab]);

  useEffect(() => {
    if (initialSection) {
      setActiveSection(initialSection);
      onConsumeInitialSection?.();
    }
  }, [initialSection, onConsumeInitialSection]);
  const [authGuideOpen, setAuthGuideOpen] = useState(false);

  const [tokenOpen, setTokenOpen] = useState(false);
  const [showToken, setShowToken] = useState(false);
  const [tokenDraft, setTokenDraft] = useState(mcp?.edgeoneApiToken ?? "");
  const [projectDraft, setProjectDraft] = useState(mcp?.edgeoneProjectName ?? "");

  const [nodeInfo, setNodeInfo] = useState<NodeRuntimeInfo | null>(null);
  const [checkingNode, setCheckingNode] = useState(false);

  useEffect(() => {
    setTokenDraft(mcp?.edgeoneApiToken ?? "");
    setProjectDraft(mcp?.edgeoneProjectName ?? "");
  }, [mcp?.edgeoneApiToken, mcp?.edgeoneProjectName]);

  const checkNode = useCallback(async () => {
    setCheckingNode(true);
    try {
      const info = await api.checkNodeRuntime();
      setNodeInfo(info);
    } catch {
      // Gracefully ignore when backend command is unavailable in standalone frontend builds
      setNodeInfo(null);
    } finally {
      setCheckingNode(false);
    }
  }, []);

  useEffect(() => {
    if (mcp?.webPage) {
      void checkNode();
    }
  }, [mcp?.webPage, checkNode]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const activeTag = document.activeElement?.tagName?.toLowerCase();
      if (activeTag === "input" || activeTag === "textarea") return;
      if (authGuideOpen) {
        setAuthGuideOpen(false);
      } else if (drawerOpen) {
        setDrawerOpen(false);
      } else {
        onGoHome();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawerOpen, authGuideOpen, onGoHome]);

  const webPageEnabled = Boolean(mcp?.webPage);
  const enabledCount = mcp
    ? [mcp.image, mcp.video, mcp.audio, mcp.search, mcp.consult, Boolean(mcp.webPage)].filter(
        Boolean,
      ).length
    : 0;

  const edgeOneDirty =
    tokenDraft.trim() !== (mcp?.edgeoneApiToken ?? "").trim() ||
    projectDraft.trim() !== (mcp?.edgeoneProjectName ?? "").trim();

  const openTheaterDrawer = (tab: McpToolKey) => {
    setDrawerTab(tab);
    setDrawerOpen(true);
  };

  const handleToggleWebPageSwitch = async (next: boolean) => {
    await onToggleWebPage(next);
    if (next) {
      void checkNode();
    }
  };

  const nodeMissingOrOld =
    nodeInfo !== null && (!nodeInfo.available || !nodeInfo.meetsMinVersion);
  const nodeReady = nodeInfo !== null && nodeInfo.available && nodeInfo.meetsMinVersion;

  return (
    <div className="w-full px-4 sm:px-5 py-4 flex flex-col gap-5">
      {/* Page Intro */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="min-w-0">
          <div className="text-[11px] font-bold tracking-[0.08em] uppercase text-[var(--accent)] mb-1">
            Tako 超级 MCP · 按需启用
          </div>
          <h1 className="text-[20px] font-semibold tracking-[-0.025em] text-[var(--text-primary)] leading-tight">
            MCP 能力中心
          </h1>
          <p className="text-[12.5px] text-[var(--text-secondary)] mt-1">
            开启的能力自动同步至 Tako 模式客户端；点击卡片即可在右侧抽屉体验演示。
          </p>
        </div>

        <div className="flex items-center gap-3 shrink-0">
          {activeSection === "builtin" && (
            <>
              <span className="text-[12px] text-[var(--text-muted)] tabular-nums">
                <strong className="text-[var(--text-primary)] font-semibold">{enabledCount}</strong> / 6 已开启
              </span>
              <button
                type="button"
                data-testid="open-mcp-demo"
                onClick={() => openTheaterDrawer(drawerTab)}
                className="h-[32px] px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer"
              >
                <Sparkles className="h-3.5 w-3.5 text-[var(--accent)]" />
                <span>效果演示</span>
              </button>
            </>
          )}
        </div>
      </div>

      {/* Section Switcher: Built-in Tako MCP | Cross-Platform MCP Sync | Cross-Platform Skills Sync */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SegmentedControl<McpCenterSection>
          value={activeSection}
          onChange={setActiveSection}
          options={[
            { id: "builtin", label: "Tako 内置工具" },
            { id: "mcp-sync", label: "多平台 MCP 同步" },
            { id: "skill-sync", label: "多平台 Skill 同步" },
          ]}
        />
      </div>

      {activeSection === "mcp-sync" && (
        <MultiPlatformSyncPanel mode="mcp" onNotify={onNotify} />
      )}

      {activeSection === "skill-sync" && (
        <MultiPlatformSyncPanel mode="skill" onNotify={onNotify} />
      )}

      {/* Capabilities Grid: Full-width Tako Web Page + 2-Column 5 Core Abilities */}
      {activeSection === "builtin" && (
      <div className="grid grid-cols-1 min-[640px]:grid-cols-2 gap-3">
        {/* Featured Full-Width Card: Tako Web Page (Tencent Cloud EdgeOne) */}
        <article
          className="min-[640px]:col-span-2 rounded-[13px] border border-[var(--accent-border)] p-4 flex flex-col gap-3"
          style={{ background: "var(--webpage-grad)" }}
        >
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2.5">
                <span className="h-8 w-8 rounded-[9px] bg-[var(--accent)] text-white inline-flex items-center justify-center shrink-0">
                  <Globe2 className="h-4 w-4" strokeWidth={1.8} />
                </span>
                <div className="flex items-center gap-2 flex-wrap">
                  <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">
                    网站上线 · Tako Web Page
                  </h3>
                  <span className="text-[10.5px] font-semibold px-2 py-0.5 rounded-full bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[var(--accent)]">
                    腾讯云 EdgeOne
                  </span>
                </div>
              </div>

              <p className="text-[12px] text-[var(--text-secondary)] mt-2.5 leading-relaxed">
                AI 完成网站后，浏览器确认一次授权，即可发布到腾讯云 EdgeOne 公网。
              </p>
            </div>

            <div className="flex items-center gap-2.5 shrink-0 pt-0.5">
              <button
                type="button"
                onClick={() => setAuthGuideOpen(true)}
                className="h-[32px] px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] transition cursor-pointer"
              >
                查看授权流程
              </button>
              <ToggleSwitch
                checked={webPageEnabled}
                disabled={savingMcp || !mcp}
                ariaLabel="开启网站上线 · Tako Web Page"
                data-testid="mcp-toggle-webpage"
                onChange={(next) => void handleToggleWebPageSwitch(next)}
              />
            </div>
          </div>

          {/* Node.js >= 18 Runtime Check Hint */}
          {webPageEnabled && nodeMissingOrOld && (
            <div className="rounded-[10px] border border-[var(--warning-border)] bg-[var(--warning-soft)] px-3.5 py-2.5 flex flex-wrap items-center justify-between gap-2 text-[11.5px] text-[var(--warning)]">
              <div className="flex items-center gap-2 min-w-0 flex-1">
                <AlertTriangle className="h-4 w-4 shrink-0" />
                <span className="leading-snug">
                  {nodeInfo.available
                    ? `当前 Node.js 版本为 ${nodeInfo.version ?? "未知"}，网站上线需要 Node.js 18 及以上版本。`
                    : "未检测到本机 Node.js 环境，使用网站上线需要安装 Node.js 18+。"}
                </span>
              </div>
              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  onClick={() => void api.openUrl("https://nodejs.org/")}
                  className="h-7 px-2.5 rounded-[7px] bg-[var(--bg-card)] border border-[var(--warning-border)] text-[var(--warning)] text-[11.5px] font-semibold inline-flex items-center gap-1 cursor-pointer"
                >
                  <span>下载 Node.js</span>
                  <ExternalLink className="h-3 w-3" />
                </button>
                <button
                  type="button"
                  disabled={checkingNode}
                  onClick={() => void checkNode()}
                  className="h-7 px-2 rounded-[7px] hover:bg-[var(--bg-card)]/60 text-[11px] inline-flex items-center gap-1 cursor-pointer disabled:opacity-50"
                >
                  <RefreshCw className={`h-3 w-3 ${checkingNode ? "animate-spin" : ""}`} />
                  <span>重检</span>
                </button>
              </div>
            </div>
          )}

          {webPageEnabled && nodeReady && (
            <div className="flex items-center gap-1.5 text-[11.5px] text-[var(--success)]">
              <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
              <span>
                本机 Node.js {nodeInfo.version} 已就绪 · 已自动同步至 Tako 模式客户端
              </span>
            </div>
          )}

          {/* Collapsible Advanced: Optional Token & Project Name */}
          <div className="pt-2 border-t border-[var(--accent-border)]/70">
            <button
              type="button"
              onClick={() => setTokenOpen((prev) => !prev)}
              className="inline-flex items-center gap-1 text-[11.5px] font-semibold text-[var(--accent)] hover:underline cursor-pointer"
            >
              <span>高级：Token 与项目名（选填）</span>
              <ChevronDown
                className={`h-3.5 w-3.5 transition-transform ${tokenOpen ? "rotate-180" : ""}`}
              />
            </button>

            {tokenOpen && (
              <div className="mt-3 flex flex-col gap-2.5">
                <p className="text-[11px] text-[var(--text-muted)]">
                  留空即可免密钥浏览器授权；仅在团队固定账号或需要指定发布项目名时选填。
                </p>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                  <div className="flex flex-col gap-1">
                    <label className="text-[11px] font-medium text-[var(--text-secondary)]">
                      EDGEONE_PAGES_API_TOKEN
                    </label>
                    <div className="relative">
                      <input
                        type={showToken ? "text" : "password"}
                        value={tokenDraft}
                        onChange={(e) => setTokenDraft(e.target.value)}
                        placeholder="留空即可免密钥浏览器授权"
                        className="w-full h-[33px] pl-2.5 pr-8 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                      />
                      <button
                        type="button"
                        onClick={() => setShowToken((v) => !v)}
                        aria-label={showToken ? "隐藏 Token" : "显示 Token"}
                        className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--text-muted)] hover:text-[var(--text-primary)] cursor-pointer"
                      >
                        {showToken ? (
                          <EyeOff className="h-3.5 w-3.5" />
                        ) : (
                          <Eye className="h-3.5 w-3.5" />
                        )}
                      </button>
                    </div>
                  </div>

                  <div className="flex flex-col gap-1">
                    <label className="text-[11px] font-medium text-[var(--text-secondary)]">
                      EDGEONE_PAGES_PROJECT_NAME
                    </label>
                    <input
                      type="text"
                      value={projectDraft}
                      onChange={(e) => setProjectDraft(e.target.value)}
                      placeholder="例如：portfolio-site（留空自动生成）"
                      className="w-full h-[33px] px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                    />
                  </div>
                </div>

                <div className="flex justify-end">
                  <button
                    type="button"
                    disabled={!edgeOneDirty || savingMcp}
                    onClick={() =>
                      void onSaveEdgeOneConfig(tokenDraft.trim(), projectDraft.trim())
                    }
                    className="h-7 px-3 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11.5px] font-semibold cursor-pointer disabled:opacity-45"
                  >
                    {savingMcp ? "保存中..." : "保存配置"}
                  </button>
                </div>
              </div>
            )}
          </div>
        </article>

        {/* 5 Clean Core Capability Cards */}
        {CORE_MCP_CARDS.map(({ key, mcpToolAttr, title, description, Icon }) => {
          const enabled = Boolean(mcp?.[key]);
          return (
            <article
              key={key}
              data-mcp-tool={mcpToolAttr}
              className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] p-4 min-h-[126px] flex flex-col transition"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="h-8 w-8 rounded-[9px] bg-[var(--bg-subtle)] text-[var(--text-secondary)] inline-flex items-center justify-center shrink-0">
                  <Icon className="h-[17px] w-[17px]" strokeWidth={1.7} />
                </span>
                <ToggleSwitch
                  checked={enabled}
                  disabled={savingMcp || !mcp}
                  ariaLabel={`切换${title}开关`}
                  data-testid={`toggle-mcp-${key}`}
                  onChange={() => onToggleCoreTool(key)}
                />
              </div>

              <h3 className="text-[13.5px] font-semibold text-[var(--text-primary)] mt-3">
                {title}
              </h3>
              <p className="text-[11.5px] text-[var(--text-muted)] mt-1 leading-relaxed">
                {description}
              </p>

              <div className="mt-auto pt-3 flex items-center justify-between gap-2">
                <StatusPill tone={enabled ? "ok" : "neutral"}>
                  {enabled ? "已开启" : "已关闭"}
                </StatusPill>
                <button
                  type="button"
                  data-testid={`open-mcp-demo-${key}`}
                  onClick={() => openTheaterDrawer(key)}
                  className="text-[11.5px] font-semibold text-[var(--accent)] hover:underline cursor-pointer"
                >
                  体验能力 →
                </button>
              </div>
            </article>
          );
        })}
      </div>
      )}

      {/* EdgeOne Authorization Flow Modal */}
      {authGuideOpen && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
          onClick={() => setAuthGuideOpen(false)}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[440px] rounded-[15px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] overflow-hidden"
          >
            <div className="px-5 py-3.5 border-b border-[var(--border)] flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Globe2 className="h-4 w-4 text-[var(--accent)]" />
                <h2 className="text-[14px] font-semibold text-[var(--text-primary)]">
                  网站上线 · 免密钥浏览器授权流程
                </h2>
              </div>
              <button
                type="button"
                aria-label="关闭授权流程说明"
                onClick={() => setAuthGuideOpen(false)}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="px-5 py-4 flex flex-col gap-3 text-[12.5px] text-[var(--text-secondary)]">
              <div className="rounded-[10px] bg-[var(--bg-subtle)]/80 px-3.5 py-2.5 text-[12px] text-[var(--text-primary)]">
                默认无需申请或粘贴任何 API Token，按以下三步即可把 AI 写的网页上线到公网：
              </div>
              <ol className="space-y-2.5 pl-4 list-decimal leading-relaxed">
                <li>
                  <strong className="text-[var(--text-primary)]">开启开关</strong>：在卡片右上角开启「网站上线」，自动同步至所有 Tako 模式桌面客户端。
                </li>
                <li>
                  <strong className="text-[var(--text-primary)]">浏览器确认一次授权</strong>：让 AI 发布网站时，系统会自动打开腾讯云 EdgeOne 授权页，点一次确认即可完成认证。
                </li>
                <li>
                  <strong className="text-[var(--text-primary)]">获得公网地址</strong>：静态页面自动打包部署，几秒内返回可公开访问的 HTTPS 链接。
                </li>
              </ol>
              <p className="text-[11px] text-[var(--text-muted)]">
                提示：运行该能力需要本机已安装 Node.js 18 及以上版本；如需绑定固定项目或团队 Token，可在卡片下方「高级」折叠区选填。
              </p>
            </div>

            <div className="px-5 py-3 border-t border-[var(--border)] flex items-center justify-end gap-2">
              {!webPageEnabled && (
                <button
                  type="button"
                  disabled={savingMcp || !mcp}
                  onClick={() => {
                    setAuthGuideOpen(false);
                    void handleToggleWebPageSwitch(true);
                  }}
                  className="h-8 px-3.5 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold cursor-pointer disabled:opacity-50"
                >
                  立即开启能力
                </button>
              )}
              <button
                type="button"
                onClick={() => setAuthGuideOpen(false)}
                className="h-8 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
              >
                知道了
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Right Slide-Over Drawer for McpInlineTheater */}
      {drawerOpen && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex justify-end"
          onClick={() => setDrawerOpen(false)}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className="drawer-enter w-[min(680px,94vw)] h-full bg-[var(--bg-canvas)] border-l border-[var(--border)] shadow-[var(--shadow-modal)] flex flex-col overflow-hidden"
          >
            <div className="h-[48px] shrink-0 px-4 border-b border-[var(--border)] bg-[var(--bg-card)] flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0" />
                <span className="text-[13.5px] font-semibold text-[var(--text-primary)] truncate">
                  MCP 能力体验剧场
                </span>
              </div>
              <button
                type="button"
                aria-label="关闭能力体验抽屉"
                onClick={() => setDrawerOpen(false)}
                className="h-7 w-7 rounded-[7px] text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] inline-flex items-center justify-center cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="flex-1 overflow-y-auto subtle-scroll p-4">
              <McpInlineTheater
                open={drawerOpen}
                activeTab={drawerTab}
                mcp={mcp}
                savingMcp={savingMcp}
                loggedIn={loggedIn}
                loggingIn={loggingIn}
                onSelectTab={(tab) => setDrawerTab(tab)}
                onToggleMcp={(key) => onToggleCoreTool(key)}
                onSelectConsultModel={onSelectConsultModel}
                onRequestLogin={onRequestLogin}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
