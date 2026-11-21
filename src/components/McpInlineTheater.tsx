import { useEffect, useMemo, useRef, useState } from "react";
import {
  Brain,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  ExternalLink,
  Film,
  Globe,
  Headphones,
  Image as ImageIcon,
  Layers,
  Loader2,
  Pause,
  Play,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  SplitSquareHorizontal,
  Volume2,
  Zap,
} from "lucide-react";
import {
  api,
  type LiveDemoResult,
  type McpToolsConfig,
} from "../lib/api";

export type McpToolKey = "image" | "video" | "audio" | "search" | "consult";
type TheaterMode = "showcase" | "live";
type AudioEmotionKey = "warm" | "lively" | "radio";

interface McpInlineTheaterProps {
  open: boolean;
  activeTab: McpToolKey;
  mcp: McpToolsConfig | null;
  savingMcp: boolean;
  loggedIn: boolean;
  loggingIn: boolean;
  onSelectTab: (tab: McpToolKey) => void;
  onToggleMcp: (key: McpToolKey) => void;
  onSelectConsultModel: (model: string) => void;
  onRequestLogin: () => void;
}

interface ToolMeta {
  key: McpToolKey;
  label: string;
  Icon: typeof ImageIcon;
  agentAction: string;
  agentDone: string;
  defaultLivePrompt: string;
}

const TOOL_METAS: ToolMeta[] = [
  {
    key: "image",
    label: "生图",
    Icon: ImageIcon,
    agentAction: "正在渲染高清静物画布...",
    agentDone: "已生成并完成本地落盘 · 1.6s",
    defaultLivePrompt:
      "在原木桌面拍一张磨砂奶油白陶瓷手冲壶静物图，清晨斜射阳光，电影感浅景深",
  },
  {
    key: "video",
    label: "生视频",
    Icon: Film,
    agentAction: "正在插值运镜并合成动态镜头...",
    agentDone: "5s 动态成片已渲染完毕 · 2.4s",
    defaultLivePrompt:
      "奇幻空中花园里的瀑布缓缓倾泻，柔和云雾飘过悬浮石阶，花瓣随微风轻摇，慢速电影推镜",
  },
  {
    key: "audio",
    label: "声音",
    Icon: Headphones,
    agentAction: "正在合成「冰糖」情绪真声...",
    agentDone: "高表现力配音已就绪 · 1.1s",
    defaultLivePrompt:
      "在 Tako Switch 一键挂载之后，写代码和做原型都能直接呼出自然有温度的旁白配音。",
  },
  {
    key: "search",
    label: "联网",
    Icon: Globe,
    agentAction: "正在实时抓取权威技术信源...",
    agentDone: "已核验 3 条真实来源并完成归纳 · 1.4s",
    defaultLivePrompt:
      "2026 年 macOS Sequoia 应用签名公证与增量自动更新有哪些必须注意的新要点？",
  },
  {
    key: "consult",
    label: "外脑",
    Icon: Brain,
    agentAction: "正在发起跨模型独立交叉会诊...",
    agentDone: "独立审阅完成 · 发现 3 处关键边界 · 1.9s",
    defaultLivePrompt:
      "本地多端代理网关在 18419 端口被异常占用或客户端热切换时，如何做到零感知自愈与幂等不丢请求？",
  },
];

interface SceneItem {
  title: string;
  prompt: string;
  elapsedLabel: string;
}

const TOOL_SCENES: Record<McpToolKey, SceneItem[]> = {
  image: [
    {
      title: "陶瓷釉色重塑",
      prompt:
        "把桌面这只暖陶橘马克杯的釉面改成森林墨绿陶瓷质感，保留窗口自然光影和木纹桌面。",
      elapsedLabel: "1.6s",
    },
    {
      title: "双参考图跨风格合成",
      prompt:
        "参考 @image0 的玻璃温室轮廓和 @image1 的林间光影，合成一幅林中玻璃木屋跨风格概念图。",
      elapsedLabel: "2.1s",
    },
  ],
  video: [
    {
      title: "奇幻空中花园成片",
      prompt:
        "以这张空中花园概念图为首帧，生成 5 秒慢推进镜头：瀑布水流倾泻、云雾流动、花瓣微摇。",
      elapsedLabel: "2.4s",
    },
    {
      title: "首帧对比与尾帧续写",
      prompt:
        "保持当前画面构图与角色光照一致，对照首帧原图延展 5 秒动态运镜，超时自动凭任务号续传。",
      elapsedLabel: "2.8s",
    },
  ],
  audio: [
    {
      title: "冰糖多情绪对比",
      prompt:
        "用「冰糖」声音分别以温柔叙述、灵动解说和深夜电台三种语气，朗读这段产品开场旁白。",
      elapsedLabel: "1.1s",
    },
    {
      title: "情绪微调与字幕同步",
      prompt:
        "切换深夜电台沉静语调，在最后一句话放慢节奏并保留自然呼吸感，输出可同步字幕音频。",
      elapsedLabel: "1.3s",
    },
  ],
  search: [
    {
      title: "2026 主流 AI 桌面编程工具能力横评",
      prompt:
        "对比 2026 年主流桌面 AI 编程工具在本地代理、MCP 扩展与多模型协同上的核心差异，附真实信源。",
      elapsedLabel: "1.4s",
    },
    {
      title: "macOS Sequoia 应用公证与增量更新实操要点",
      prompt:
        "检索 macOS Sequoia 下桌面应用 Hardened Runtime 公证与 Tauri 增量签名更新的官方规范。",
      elapsedLabel: "1.5s",
    },
  ],
  consult: [
    {
      title: "高并发支付回调幂等与防重扣盲点挑刺",
      prompt:
        "异步任务 90 秒超时重试与支付回调并发到达时，怎样避免重复扣费和状态机回退？请独立挑刺。",
      elapsedLabel: "1.9s",
    },
    {
      title: "本地多端代理网关端口自愈与零感切换评审",
      prompt:
        "多个桌面客户端同时切入本地统一网关时，端口抢占与配置热更新有哪些竞态盲点？",
      elapsedLabel: "1.8s",
    },
  ],
};

interface AudioTrack {
  key: AudioEmotionKey;
  label: string;
  badge: string;
  src: string;
  phrases: [string, string, string];
}

const AUDIO_TRACKS: AudioTrack[] = [
  {
    key: "warm",
    label: "🌤 温柔叙述",
    badge: "冰糖 · 温柔叙述",
    src: "/mcp-demo/voice-bingtang-warm.mp3",
    phrases: [
      "在 Tako Switch 一键挂载之后，",
      "不管是写方案、做产品原型，",
      "还是给视频配一段有温度的旁白，一句话就能直接合成出来。",
    ],
  },
  {
    key: "lively",
    label: "⚡ 灵动解说",
    badge: "冰糖 · 灵动解说",
    src: "/mcp-demo/voice-bingtang-lively.mp3",
    phrases: [
      "嗨！现在的演示已经不是冷冰冰的参数表啦，",
      "你可以直接左右拖动改图滑块，",
      "或者改一句词，马上体验现场真跑的效果！",
    ],
  },
  {
    key: "radio",
    label: "🌙 深夜电台",
    badge: "冰糖 · 深夜电台",
    src: "/mcp-demo/voice-bingtang-radio.mp3",
    phrases: [
      "深夜改完最后一行代码，窗外正下着细雨。",
      "把繁琐的配置留给本地网关，",
      "把专注留给你真正想创造的作品。",
    ],
  },
];

const WAVEFORM_28 = [
  24, 42, 68, 52, 86, 62, 38, 76, 94, 66, 46, 82, 92, 56,
  40, 74, 88, 50, 64, 44, 78, 90, 58, 42, 70, 54, 36, 52,
];

interface SearchSegment {
  text: string;
  citationIdx?: number;
}

interface SearchSceneData {
  summarySegments: SearchSegment[];
  citations: {
    domain: string;
    title: string;
    snippet: string;
    url: string;
  }[];
}

const SEARCH_SCENES: SearchSceneData[] = [
  {
    summarySegments: [
      {
        text: "2026 年主流 AI 桌面编程工具已全面转向「本地工具链 + 多模态 MCP」双轨架构。Claude Desktop 与 Codex App 均支持通过标准 MCP 协议接入图像、音视频与实时联网能力",
      },
      { text: "[1]", citationIdx: 0 },
      {
        text: "；工程实践表明，将媒体生成与网页检索收敛到本地统一网关后，多客户端可共享密钥、额度统计与本地落盘目录，显著减少重复配置",
      },
      { text: "[2]", citationIdx: 1 },
      {
        text: "；同时，跨模型交叉会诊正在成为复杂并发与架构排障的标准工作流",
      },
      { text: "[3]", citationIdx: 2 },
      { text: "。" },
    ],
    citations: [
      {
        domain: "modelcontextprotocol.io",
        title: "Model Context Protocol · Specification & Tool Ecosystem",
        snippet:
          "Standardized JSON-RPC interface enabling desktop AI clients to invoke local and gateway-backed tools seamlessly.",
        url: "https://modelcontextprotocol.io/introduction",
      },
      {
        domain: "developers.openai.com",
        title: "Codex & Local MCP Integration Guide",
        snippet:
          "Configure stdio and HTTP MCP servers in ~/.codex/config.toml so agent workflows inherit unified tool permissions.",
        url: "https://developers.openai.com/codex/mcp",
      },
      {
        domain: "docs.anthropic.com",
        title: "Building Effective Agents · Cross-Model Verification",
        snippet:
          "Evaluator-optimizer and independent reviewer patterns reduce blind spots in high-stakes architecture changes.",
        url: "https://docs.anthropic.com/en/docs/build-with-claude/agent-patterns",
      },
    ],
  },
  {
    summarySegments: [
      {
        text: "macOS Sequoia 分发桌面客户端时，必须开启 Hardened Runtime 并通过 notarytool 完成 Apple 公证，避免首次启动触发 Gatekeeper 拦截",
      },
      { text: "[1]", citationIdx: 0 },
      {
        text: "；使用 Tauri v2 内置 Updater 时，需在构建阶段生成 minisign 签名并校验 latest.json 中的平台产物摘要，才能实现无感增量热更新",
      },
      { text: "[2]", citationIdx: 1 },
      {
        text: "；若应用内嵌本地音视频素材，还应在 CSP 中显式放行 media-src 'self' data:",
      },
      { text: "[3]", citationIdx: 2 },
      { text: "。" },
    ],
    citations: [
      {
        domain: "developer.apple.com",
        title: "Notarizing macOS Software Before Distribution",
        snippet:
          "Submit signed app archives via xcrun notarytool and staple the ticket so Gatekeeper verifies offline launches.",
        url: "https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution",
      },
      {
        domain: "v2.tauri.app",
        title: "Updater Plugin · Tauri v2 Signed Updates",
        snippet:
          "Verify update payloads using Ed25519 minisign keys and publish platform artifact metadata via latest.json.",
        url: "https://v2.tauri.app/plugin/updater/",
      },
      {
        domain: "v2.tauri.app",
        title: "Content Security Policy (CSP) | Tauri v2 Security",
        snippet:
          "Add media-src 'self' data: in tauri.conf.json to allow bundled local audio and video elements safely.",
        url: "https://v2.tauri.app/security/csp/",
      },
    ],
  },
];

interface ConsultCard {
  tag: string;
  tone: "warning" | "danger" | "success";
  title: string;
  detail: string;
}

interface ConsultSceneData {
  question: string;
  cards: ConsultCard[];
}

const CONSULT_SCENES: ConsultSceneData[] = [
  {
    question:
      "异步生视频 90 秒超时重试与支付回调并发到达时，当前状态机容易在哪些地方造成重复扣费或状态回退？",
    cards: [
      {
        tag: "核心盲点",
        tone: "warning",
        title: "重试缺少稳定幂等键",
        detail:
          "客户端超时自动重发若生成新 request_id，网关会当作全新生视频任务再次扣费；必须将 idempotency_key 绑定到单次用户意图。",
      },
      {
        tag: "极端边界",
        tone: "danger",
        title: "settled 态被晚到回调覆写",
        detail:
          "若轮询已结算为 completed，晚到的失败或超时回调若无状态单调锁（WHERE status IN ('pending','running')），会触发错误退款。",
      },
      {
        tag: "推荐改法",
        tone: "success",
        title: "超时下发 taskId 走只读续传",
        detail:
          "超时立刻返回 status: 'pending' + taskId，后续点击或工具调用携带 resumeTaskId 走纯查询通路，彻底零重扣。",
      },
    ],
  },
  {
    question:
      "Codex、Claude 与 Antigravity 同时切入本地网关时，端口自愈与配置切换怎样做到无感且不丢活跃连接？",
    cards: [
      {
        tag: "核心盲点",
        tone: "warning",
        title: "裸杀进程导致长连接中断",
        detail:
          "多端并发探测 18419 时若各自尝试重启子进程，会撕裂正在流式输出的 SSE/视频任务连接。",
      },
      {
        tag: "极端边界",
        tone: "danger",
        title: "旧配置与热切换写竞争",
        detail:
          "并行写入客户端配置文件时缺乏原子临时文件 + rename，极易出现半截 JSON/TOML 损坏原配置。",
      },
      {
        tag: "推荐改法",
        tone: "success",
        title: "单飞探活锁 + 原子写盘",
        detail:
          "网关拉起加进程级互斥锁并在健康探针通过后再放行流量，所有宿主配置改写强制走备份一次 + 原子替换。",
      },
    ],
  },
];

const CONSULT_MODEL_PRESETS = [
  "gpt-6.2-sol",
  "gpt-6.2-astra",
  "claude-opus-5-5",
  "deepseek-v4.1-flash",
  "swe-2",
] as const;

const IMAGE_MODELS = [
  "gpt-image-2",
  "gemini-3.1-flash-image",
  "grok-imagine-image",
] as const;

const AUDIO_VOICES = ["冰糖", "茉莉", "苏打", "白桦"] as const;
const AUDIO_TONES = ["温柔细腻", "轻快活力", "沉静电台"] as const;
const SEARCH_ENGINES = ["deepseek", "grok"] as const;

function formatAudioTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

export function McpInlineTheater({
  open,
  activeTab,
  mcp,
  savingMcp,
  loggedIn,
  loggingIn,
  onSelectTab,
  onToggleMcp,
  onSelectConsultModel,
  onRequestLogin,
}: McpInlineTheaterProps) {
  const [mode, setMode] = useState<TheaterMode>("showcase");
  const [sceneIdxByTab, setSceneIdxByTab] = useState<Record<McpToolKey, number>>({
    image: 0,
    video: 0,
    audio: 0,
    search: 0,
    consult: 0,
  });

  // Agent stream pulse & replay trigger
  const [replayNonce, setReplayNonce] = useState(0);
  const [agentPhase, setAgentPhase] = useState<"streaming" | "done">("done");

  // Image before/after slider + multi-ref toggle
  const [sliderPos, setSliderPos] = useState(50);
  const [multiRefView, setMultiRefView] = useState<"comparison" | "cabin">("comparison");
  const hasWiggledRef = useRef(false);
  const wiggleCancelRef = useRef<(() => void) | null>(null);

  // Video state
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [videoPlaying, setVideoPlaying] = useState(true);
  const [videoCurrentTime, setVideoCurrentTime] = useState(0);
  const [videoDuration, setVideoDuration] = useState(5.0);
  const [compareVideoFrame, setCompareVideoFrame] = useState(false);

  // Audio state
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [audioEmotion, setAudioEmotion] = useState<AudioEmotionKey>("warm");
  const [audioPlaying, setAudioPlaying] = useState(false);
  const [audioCurrentTime, setAudioCurrentTime] = useState(0);
  const [audioDuration, setAudioDuration] = useState(11.0);
  const [audioError, setAudioError] = useState<string | null>(null);

  // Search typewriter & citation highlight
  const [searchRevealChars, setSearchRevealChars] = useState(9999);
  const [activeCitationIdx, setActiveCitationIdx] = useState<number | null>(null);

  // Consult progressive card reveal
  const [consultRevealCount, setConsultRevealCount] = useState(3);

  // Copy states
  const [copiedPrompt, setCopiedPrompt] = useState(false);
  const [copiedSavedPath, setCopiedSavedPath] = useState(false);
  const copyTimerRef = useRef<number | null>(null);
  const savedCopyTimerRef = useRef<number | null>(null);

  // Live run mode state
  const [livePrompts, setLivePrompts] = useState<Record<McpToolKey, string>>({
    image: TOOL_METAS[0].defaultLivePrompt,
    video: TOOL_METAS[1].defaultLivePrompt,
    audio: TOOL_METAS[2].defaultLivePrompt,
    search: TOOL_METAS[3].defaultLivePrompt,
    consult: TOOL_METAS[4].defaultLivePrompt,
  });
  const [liveImageModel, setLiveImageModel] = useState<string>("gpt-image-2");
  const [liveAspectRatio, setLiveAspectRatio] = useState<"1:1" | "16:9">("1:1");
  const [liveAudioVoice, setLiveAudioVoice] = useState<string>("冰糖");
  const [liveAudioTone, setLiveAudioTone] = useState<string>("温柔细腻");
  const [liveSearchEngine, setLiveSearchEngine] = useState<"deepseek" | "grok">("deepseek");
  const [runningLive, setRunningLive] = useState(false);
  const [liveElapsedMs, setLiveElapsedMs] = useState(0);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [liveResults, setLiveResults] = useState<Partial<Record<McpToolKey, LiveDemoResult>>>({});

  const currentMeta = TOOL_METAS.find((t) => t.key === activeTab) ?? TOOL_METAS[0];
  const currentSceneIdx = sceneIdxByTab[activeTab] ?? 0;
  const currentScenes = TOOL_SCENES[activeTab];
  const currentScene = currentScenes[currentSceneIdx] ?? currentScenes[0];
  const currentTrack = AUDIO_TRACKS.find((t) => t.key === audioEmotion) ?? AUDIO_TRACKS[0];
  const consultModel = mcp?.consultModel?.trim() || "gpt-6.2-sol";
  const isToolEnabled = Boolean(mcp?.[activeTab]);

  // Trigger 450ms agent streaming pulse + canvas reveal animations on open/tab/scene/replay
  useEffect(() => {
    if (!open || mode !== "showcase") return;

    setAgentPhase("streaming");
    const agentTimer = window.setTimeout(() => {
      setAgentPhase("done");
    }, 450);

    // Search typewriter animation
    let searchRaf: number | null = null;
    if (activeTab === "search") {
      const searchScene = SEARCH_SCENES[currentSceneIdx] ?? SEARCH_SCENES[0];
      const totalChars = searchScene.summarySegments.reduce(
        (acc, seg) => acc + seg.text.length,
        0,
      );
      setSearchRevealChars(0);
      setActiveCitationIdx(null);
      const start = performance.now();
      const durationMs = 950;
      const tick = (now: number) => {
        const progress = Math.min(1, (now - start) / durationMs);
        setSearchRevealChars(Math.floor(progress * totalChars));
        if (progress < 1) {
          searchRaf = window.requestAnimationFrame(tick);
        }
      };
      searchRaf = window.requestAnimationFrame(tick);
    }

    // Consult progressive reveal
    let consultTimers: number[] = [];
    if (activeTab === "consult") {
      setConsultRevealCount(0);
      consultTimers = [
        window.setTimeout(() => setConsultRevealCount(1), 220),
        window.setTimeout(() => setConsultRevealCount(2), 410),
        window.setTimeout(() => setConsultRevealCount(3), 580),
      ];
    }

    return () => {
      window.clearTimeout(agentTimer);
      if (searchRaf !== null) window.cancelAnimationFrame(searchRaf);
      consultTimers.forEach((t) => window.clearTimeout(t));
    };
  }, [open, mode, activeTab, currentSceneIdx, replayNonce]);

  // First-entry gentle wiggle (62 -> 42 -> 50) on image scene 0
  useEffect(() => {
    if (
      !open ||
      mode !== "showcase" ||
      activeTab !== "image" ||
      currentSceneIdx !== 0
    ) {
      return;
    }
    if (hasWiggledRef.current && replayNonce === 0) return;

    const prefersReduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    if (prefersReduced) {
      hasWiggledRef.current = true;
      setSliderPos(50);
      return;
    }

    hasWiggledRef.current = true;
    let rafId = 0;
    let cancelled = false;

    const keyframes = [
      { t: 0, v: 50 },
      { t: 0.3, v: 62 },
      { t: 0.68, v: 42 },
      { t: 1, v: 50 },
    ];
    const totalDuration = 1050;
    const startTimer = window.setTimeout(() => {
      const startTs = performance.now();
      const step = (now: number) => {
        if (cancelled) return;
        const p = Math.min(1, (now - startTs) / totalDuration);
        let val = 50;
        for (let i = 0; i < keyframes.length - 1; i++) {
          const k0 = keyframes[i];
          const k1 = keyframes[i + 1];
          if (p >= k0.t && p <= k1.t) {
            const local = (p - k0.t) / (k1.t - k0.t);
            const eased =
              local < 0.5
                ? 2 * local * local
                : 1 - Math.pow(-2 * local + 2, 2) / 2;
            val = k0.v + (k1.v - k0.v) * eased;
            break;
          }
        }
        setSliderPos(val);
        if (p < 1) {
          rafId = window.requestAnimationFrame(step);
        }
      };
      rafId = window.requestAnimationFrame(step);
    }, 240);

    wiggleCancelRef.current = () => {
      cancelled = true;
      window.clearTimeout(startTimer);
      if (rafId) window.cancelAnimationFrame(rafId);
    };

    return () => {
      wiggleCancelRef.current?.();
    };
  }, [open, mode, activeTab, currentSceneIdx, replayNonce]);

  // Pause audio when theater closes, switches away from audio tab, or enters live mode
  useEffect(() => {
    const el = audioRef.current;
    if (!el) return;
    if (!open || mode !== "showcase" || activeTab !== "audio") {
      el.pause();
      setAudioPlaying(false);
    }
  }, [open, mode, activeTab]);

  // Sync video playback when theater opens/switches to video tab
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    if (!open || mode !== "showcase" || activeTab !== "video") {
      el.pause();
      setVideoPlaying(false);
    } else {
      el.play()
        .then(() => setVideoPlaying(true))
        .catch(() => setVideoPlaying(false));
    }
  }, [open, mode, activeTab]);

  // Sync audio scene switch (scene 1 selects深夜电台 for variety)
  useEffect(() => {
    if (activeTab === "audio" && currentSceneIdx === 1) {
      setAudioEmotion("radio");
    } else if (activeTab === "audio" && currentSceneIdx === 0 && audioEmotion === "radio") {
      setAudioEmotion("warm");
    }
  }, [activeTab, currentSceneIdx]);

  // Load new MP3 when emotion track changes
  useEffect(() => {
    const el = audioRef.current;
    if (!el) return;
    const wasPlaying = audioPlaying;
    el.pause();
    el.currentTime = 0;
    setAudioCurrentTime(0);
    setAudioError(null);
    el.load();
    if (wasPlaying && open && mode === "showcase" && activeTab === "audio") {
      el.play()
        .then(() => setAudioPlaying(true))
        .catch(() => setAudioPlaying(false));
    }
  }, [audioEmotion]);

  // Live execution timer
  useEffect(() => {
    if (!runningLive) return;
    const started = performance.now();
    setLiveElapsedMs(0);
    const id = window.setInterval(() => {
      setLiveElapsedMs(performance.now() - started);
    }, 100);
    return () => window.clearInterval(id);
  }, [runningLive]);

  // Cleanup copy timers on unmount
  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
      if (savedCopyTimerRef.current !== null) window.clearTimeout(savedCopyTimerRef.current);
    };
  }, []);

  // Subtitle phrase time ranges weighted by character count
  const phraseRanges = useMemo(() => {
    const lengths = currentTrack.phrases.map((p) => Math.max(1, p.length));
    const total = lengths.reduce((a, b) => a + b, 0);
    let acc = 0;
    return lengths.map((len) => {
      const start = acc / total;
      acc += len;
      const end = acc / total;
      return { start, end };
    });
  }, [currentTrack]);

  const audioProgressRatio =
    audioDuration > 0 ? Math.min(1, Math.max(0, audioCurrentTime / audioDuration)) : 0;

  const activePhraseIdx = useMemo(() => {
    if (!audioPlaying && audioCurrentTime === 0) return -1;
    const idx = phraseRanges.findIndex(
      (r, i) =>
        audioProgressRatio >= r.start &&
        (audioProgressRatio < r.end || i === phraseRanges.length - 1),
    );
    return idx;
  }, [audioPlaying, audioCurrentTime, audioProgressRatio, phraseRanges]);

  const handleToggleAudio = async () => {
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

  const handleJumpAudioRatio = async (ratio: number) => {
    const el = audioRef.current;
    if (!el) return;
    const dur = Number.isFinite(el.duration) && el.duration > 0 ? el.duration : audioDuration;
    const target = Math.max(0, Math.min(dur, ratio * dur));
    el.currentTime = target;
    setAudioCurrentTime(target);
    if (!audioPlaying) {
      try {
        await el.play();
        setAudioPlaying(true);
      } catch {
        // ignore autoplay block
      }
    }
  };

  const handleToggleVideo = async () => {
    const el = videoRef.current;
    if (!el) return;
    if (!el.paused) {
      el.pause();
      setVideoPlaying(false);
    } else {
      try {
        await el.play();
        setVideoPlaying(true);
      } catch {
        setVideoPlaying(false);
      }
    }
  };

  const handleScrubVideo = (clientX: number, rect: DOMRect) => {
    const el = videoRef.current;
    if (!el) return;
    const dur = Number.isFinite(el.duration) && el.duration > 0 ? el.duration : videoDuration;
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
    el.currentTime = ratio * dur;
    setVideoCurrentTime(el.currentTime);
  };

  const copyText = async (text: string) => {
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
  };

  const handleCopyPrompt = async () => {
    try {
      await copyText(currentScene.prompt);
    } catch {
      // show feedback even if clipboard permission mocked
    }
    setCopiedPrompt(true);
    if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
    copyTimerRef.current = window.setTimeout(() => setCopiedPrompt(false), 1800);
  };

  const handleCopySavedPath = async (path: string) => {
    try {
      await copyText(path);
    } catch {
      // ignore
    }
    setCopiedSavedPath(true);
    if (savedCopyTimerRef.current !== null) window.clearTimeout(savedCopyTimerRef.current);
    savedCopyTimerRef.current = window.setTimeout(() => setCopiedSavedPath(false), 1800);
  };

  const handleJumpLive = () => {
    setLivePrompts((prev) => ({ ...prev, [activeTab]: currentScene.prompt }));
    setLiveError(null);
    setMode("live");
  };

  const handleRunLiveDemo = async (resumeTaskId?: string) => {
    if (runningLive) return;
    const prompt = livePrompts[activeTab].trim();
    if (!prompt && !resumeTaskId) return;

    setRunningLive(true);
    setLiveError(null);
    try {
      const result = await api.runMcpLiveDemo({
        tool: activeTab,
        prompt,
        model:
          activeTab === "image"
            ? liveImageModel
            : activeTab === "consult"
              ? consultModel
              : undefined,
        aspectRatio: activeTab === "image" ? liveAspectRatio : undefined,
        voice: activeTab === "audio" ? liveAudioVoice : undefined,
        instructions: activeTab === "audio" ? liveAudioTone : undefined,
        provider: activeTab === "search" ? liveSearchEngine : undefined,
        resumeTaskId,
      });
      setLiveResults((prev) => ({ ...prev, [activeTab]: result }));
    } catch (err) {
      setLiveError(String(err));
    } finally {
      setRunningLive(false);
    }
  };

  const currentLiveResult = liveResults[activeTab];
  const searchScene = SEARCH_SCENES[currentSceneIdx] ?? SEARCH_SCENES[0];
  const consultScene = CONSULT_SCENES[currentSceneIdx] ?? CONSULT_SCENES[0];

  // Render typewriter text with interactive [1][2][3] citation badges
  const renderSearchSummary = () => {
    let remaining = searchRevealChars;
    return searchScene.summarySegments.map((seg, idx) => {
      if (remaining <= 0) return null;
      const visibleText = seg.text.slice(0, remaining);
      remaining -= seg.text.length;

      if (seg.citationIdx !== undefined && visibleText === seg.text) {
        const cIdx = seg.citationIdx;
        const highlighted = activeCitationIdx === cIdx;
        return (
          <button
            key={idx}
            type="button"
            onMouseEnter={() => setActiveCitationIdx(cIdx)}
            onMouseLeave={() => setActiveCitationIdx(null)}
            onClick={() => setActiveCitationIdx((prev) => (prev === cIdx ? null : cIdx))}
            className={`inline-flex items-center justify-center mx-0.5 px-1.5 py-0 rounded-full text-[10.5px] font-semibold align-baseline transition cursor-pointer ${
              highlighted
                ? "bg-[var(--accent)] text-white shadow-sm scale-105"
                : "bg-[var(--accent-soft)] text-[var(--accent)] border border-[var(--accent-border)] hover:bg-[var(--accent)] hover:text-white"
            }`}
          >
            {visibleText}
          </button>
        );
      }
      return <span key={idx}>{visibleText}</span>;
    });
  };

  const videoDur = videoDuration > 0 ? videoDuration : 5.0;
  const videoProgressPct = Math.min(100, Math.max(0, (videoCurrentTime / videoDur) * 100));

  return (
    <div
      data-testid="mcp-inline-theater"
      className="relative rounded-[14px] border border-[var(--accent-border)]/70 bg-gradient-to-br from-[var(--bg-canvas)]/95 via-[var(--bg-card)]/95 to-[var(--accent-soft)]/50 backdrop-blur-md p-3 sm:p-3.5 flex flex-col gap-3 min-w-0 max-w-full overflow-hidden shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
    >
      {/* Top Lightweight Navigation Bar */}
      <div className="flex flex-wrap items-center justify-between gap-2 min-w-0">
        {/* Left: 5 Tool Capsules + Light Mount Status Hint */}
        <div className="flex flex-wrap items-center gap-1.5 min-w-0">
          <div className="inline-flex flex-wrap items-center gap-1 p-0.5 rounded-full bg-[var(--bg-pill)]/90 border border-[var(--border)]">
            {TOOL_METAS.map(({ key, label, Icon }) => {
              const active = activeTab === key;
              const enabled = Boolean(mcp?.[key]);
              return (
                <button
                  key={key}
                  type="button"
                  data-testid={`mcp-demo-tab-${key}`}
                  onClick={() => onSelectTab(key)}
                  className={`px-2.5 py-1 rounded-full text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer ${
                    active
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] shadow-[var(--shadow-segment)] border border-[var(--border-strong)]/60"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)] border border-transparent"
                  }`}
                >
                  <Icon
                    className={`h-3 w-3 shrink-0 ${
                      active ? "text-[var(--accent)]" : "opacity-80"
                    }`}
                  />
                  <span>{label}</span>
                  <span
                    className={`h-1.5 w-1.5 rounded-full shrink-0 ${
                      enabled ? "bg-[var(--success)]" : "bg-[var(--border-strong)]"
                    }`}
                  />
                </button>
              );
            })}
          </div>

          <button
            type="button"
            disabled={!mcp || savingMcp}
            onClick={() => onToggleMcp(activeTab)}
            className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[11px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] bg-[var(--bg-card)]/70 border border-[var(--border)] transition cursor-pointer disabled:opacity-60"
            title="点击切换当前能力开关"
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isToolEnabled ? "bg-[var(--success)]" : "bg-[var(--border-strong)]"
              }`}
            />
            <span>{isToolEnabled ? "已挂载" : "未开启 · 点击挂载"}</span>
          </button>
        </div>

        {/* Right: Segmented Mode Switcher */}
        <div className="inline-flex items-center p-0.5 rounded-full bg-[var(--bg-pill)] border border-[var(--border)] shrink-0">
          <button
            type="button"
            data-testid="mcp-mode-showcase"
            onClick={() => setMode("showcase")}
            className={`px-2.5 py-1 rounded-full text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer ${
              mode === "showcase"
                ? "bg-[var(--bg-card)] text-[var(--text-primary)] shadow-[var(--shadow-segment)]"
                : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
            }`}
          >
            <span>✦ 实景剧场</span>
          </button>
          <button
            type="button"
            data-testid="mcp-mode-live"
            onClick={() => setMode("live")}
            className={`px-2.5 py-1 rounded-full text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer ${
              mode === "live"
                ? "bg-[var(--accent)] text-white shadow-sm"
                : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
            }`}
          >
            <span>⚡ 现场试跑</span>
          </button>
        </div>
      </div>

      {/* Mode A: Showcase Theater */}
      {mode === "showcase" ? (
        <div className="flex flex-col gap-2.5 min-w-0">
          {/* Scene Switcher + Replay Stream */}
          <div className="flex flex-wrap items-center justify-between gap-2 min-w-0">
            <div className="flex flex-wrap items-center gap-1.5 min-w-0">
              {currentScenes.map((scene, idx) => {
                const active = currentSceneIdx === idx;
                return (
                  <button
                    key={scene.title}
                    type="button"
                    data-testid={`mcp-scene-${idx}`}
                    onClick={() =>
                      setSceneIdxByTab((prev) => ({ ...prev, [activeTab]: idx }))
                    }
                    className={`px-2.5 py-1 rounded-full text-[11px] font-medium transition cursor-pointer ${
                      active
                        ? "bg-[var(--accent-soft)] text-[var(--accent)] border border-[var(--accent-border)]"
                        : "bg-[var(--bg-card)]/60 text-[var(--text-secondary)] border border-[var(--border)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    {scene.title}
                  </button>
                );
              })}
            </div>

            <button
              type="button"
              data-testid="mcp-replay-stream"
              onClick={() => setReplayNonce((n) => n + 1)}
              className="px-2.5 py-1 rounded-full text-[11px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] bg-[var(--bg-card)]/70 hover:bg-[var(--bg-card)] border border-[var(--border)] inline-flex items-center gap-1 transition cursor-pointer shrink-0"
            >
              <RotateCcw className="h-3 w-3" />
              <span>重播动效</span>
            </button>
          </div>

          {/* Simulated Conversation Flow */}
          <div className="flex flex-col gap-2 min-w-0">
            {/* User Bubble (Right) */}
            <div className="self-end max-w-[92%] sm:max-w-[82%] rounded-2xl rounded-tr-sm px-3.5 py-2 bg-[var(--btn-dark-bg)] text-[var(--btn-dark-fg)] text-[12px] leading-relaxed shadow-sm selectable">
              {currentScene.prompt}
            </div>

            {/* Agent Call Pill (Left) */}
            <div className="self-start flex items-center gap-2 max-w-full">
              <div
                className={`inline-flex items-center gap-1.5 px-3 py-1 rounded-full text-[11.5px] font-medium border transition-all duration-200 ${
                  agentPhase === "streaming"
                    ? "border-[var(--accent-border)] text-[var(--accent)] bg-gradient-to-r from-[var(--accent-soft)] via-[var(--bg-card)] to-[var(--accent-soft)] mcp-stream-shimmer"
                    : "border-[var(--success-border)] bg-[var(--success-soft)] text-[var(--success)]"
                }`}
              >
                {agentPhase === "streaming" ? (
                  <>
                    <Sparkles className="h-3 w-3 animate-pulse shrink-0" />
                    <span>✦ 正在调用 · {currentMeta.agentAction}</span>
                  </>
                ) : (
                  <>
                    <Check className="h-3 w-3 shrink-0" />
                    <span>
                      ✓ 生成完毕 · {currentScene.elapsedLabel}
                    </span>
                  </>
                )}
              </div>
            </div>
          </div>

          {/* Immersive Interactive Canvas (Borderless Fluid Stage) */}
          <div className="min-w-0">
            {/* 1. IMAGE CANVAS */}
            {activeTab === "image" && (
              currentSceneIdx === 0 ? (
                <div className="flex flex-col items-center gap-2 min-w-0">
                  <div
                    data-testid="mcp-image-slider"
                    onPointerDown={() => wiggleCancelRef.current?.()}
                    className="relative w-full max-w-[360px] aspect-square rounded-2xl overflow-hidden shadow-md border border-[var(--border)] select-none touch-none bg-[var(--bg-card)]"
                  >
                    {/* Right Base: Edited Forest Green Ceramic */}
                    <img
                      src="/mcp-demo/ceramic-mug-edited.webp"
                      alt="改图 · 森林绿釉面"
                      draggable={false}
                      className="absolute inset-0 w-full h-full object-cover"
                    />

                    {/* Left Clipped: Original Warm Terracotta */}
                    <img
                      src="/mcp-demo/ceramic-mug.webp"
                      alt="原图 · 暖陶橘"
                      draggable={false}
                      style={{
                        clipPath: `inset(0 ${100 - sliderPos}% 0 0)`,
                      }}
                      className="absolute inset-0 w-full h-full object-cover"
                    />

                    {/* Floating Corner Labels */}
                    <span className="pointer-events-none absolute top-2.5 left-2.5 z-10 px-2 py-0.5 rounded-full bg-black/55 backdrop-blur-md text-white text-[10.5px] font-medium">
                      原图 · 暖陶橘
                    </span>
                    <span className="pointer-events-none absolute top-2.5 right-2.5 z-10 px-2 py-0.5 rounded-full bg-emerald-950/70 backdrop-blur-md text-emerald-100 text-[10.5px] font-medium border border-emerald-400/30">
                      改图 · 森林绿釉面
                    </span>

                    {/* Vertical Divider & Drag Handle */}
                    <div
                      style={{ left: `${sliderPos}%` }}
                      className="pointer-events-none absolute inset-y-0 z-10 -translate-x-1/2 flex flex-col items-center justify-center"
                    >
                      <div className="w-[2px] flex-1 bg-white/90 shadow-[0_0_8px_rgba(0,0,0,0.45)]" />
                      <div className="h-8 w-8 rounded-full bg-white text-slate-900 shadow-[0_4px_14px_rgba(0,0,0,0.35)] border border-white/80 flex items-center justify-center shrink-0">
                        <ChevronLeft className="h-3.5 w-3.5 -mr-0.5" />
                        <ChevronRight className="h-3.5 w-3.5 -ml-0.5" />
                      </div>
                      <div className="w-[2px] flex-1 bg-white/90 shadow-[0_0_8px_rgba(0,0,0,0.45)]" />
                    </div>

                    {/* Accessible & Interactive Range Overlay */}
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={Math.round(sliderPos)}
                      aria-label="左右拖拽对比原图与改图滑块"
                      onChange={(e) => {
                        wiggleCancelRef.current?.();
                        setSliderPos(Number(e.target.value));
                      }}
                      className="absolute inset-0 z-20 w-full h-full opacity-0 cursor-ew-resize m-0"
                    />
                  </div>
                  <div className="text-[11px] text-[var(--text-muted)] text-center">
                    左右拖拽滑块对比釉色改图前后细节 · 光影与木纹自然保留
                  </div>
                </div>
              ) : (
                <div className="relative rounded-2xl overflow-hidden border border-[var(--border)] bg-[var(--bg-card)]/80 min-w-0">
                  <div className="relative w-full aspect-[16/10] sm:aspect-[16/9] max-h-[320px] overflow-hidden bg-black/5">
                    <img
                      src={
                        multiRefView === "comparison"
                          ? "/mcp-demo/multi-reference-comparison.webp"
                          : "/mcp-demo/forest-cabin.webp"
                      }
                      alt="双参考图跨风格合成作品"
                      className="w-full h-full object-cover transition-opacity duration-300"
                    />

                    {/* Left Floating Reference Thumbnails */}
                    <div className="absolute top-2.5 left-2.5 flex items-center gap-1.5 p-1.5 rounded-xl bg-black/50 backdrop-blur-md border border-white/15">
                      <div className="flex flex-col items-center gap-0.5">
                        <img
                          src="/mcp-demo/multi-reference-1.webp"
                          alt="@image0"
                          className="h-10 w-10 rounded-lg object-cover border border-white/25"
                        />
                        <span className="text-[9.5px] font-mono text-white/90">@image0</span>
                      </div>
                      <span className="text-white/75 text-[11px] font-bold">+</span>
                      <div className="flex flex-col items-center gap-0.5">
                        <img
                          src="/mcp-demo/multi-reference-2.webp"
                          alt="@image1"
                          className="h-10 w-10 rounded-lg object-cover border border-white/25"
                        />
                        <span className="text-[9.5px] font-mono text-white/90">@image1</span>
                      </div>
                    </div>

                    {/* Top-right Output Switcher */}
                    <div className="absolute top-2.5 right-2.5 inline-flex p-0.5 rounded-full bg-black/55 backdrop-blur-md border border-white/15">
                      <button
                        type="button"
                        onClick={() => setMultiRefView("comparison")}
                        className={`px-2.5 py-1 rounded-full text-[10.5px] font-medium transition cursor-pointer inline-flex items-center gap-1 ${
                          multiRefView === "comparison"
                            ? "bg-white text-slate-900"
                            : "text-white/80 hover:text-white"
                        }`}
                      >
                        <Layers className="h-3 w-3" />
                        <span>融合对比图</span>
                      </button>
                      <button
                        type="button"
                        onClick={() => setMultiRefView("cabin")}
                        className={`px-2.5 py-1 rounded-full text-[10.5px] font-medium transition cursor-pointer ${
                          multiRefView === "cabin"
                            ? "bg-white text-slate-900"
                            : "text-white/80 hover:text-white"
                        }`}
                      >
                        实景成片
                      </button>
                    </div>
                  </div>
                </div>
              )
            )}

            {/* 2. VIDEO CANVAS */}
            {activeTab === "video" && (
              <div className="flex flex-col gap-2 min-w-0">
                <div className="relative rounded-2xl overflow-hidden border border-[var(--border)] bg-black shadow-md min-w-0">
                  <div className="relative w-full aspect-video max-h-[320px] bg-black overflow-hidden">
                    <video
                      ref={videoRef}
                      data-testid="mcp-demo-video"
                      src="/mcp-demo/floating-garden-hq.mp4"
                      poster="/mcp-demo/floating-garden.webp"
                      autoPlay
                      loop
                      muted
                      playsInline
                      onPlay={() => setVideoPlaying(true)}
                      onPause={() => setVideoPlaying(false)}
                      onLoadedMetadata={(e) => {
                        if (Number.isFinite(e.currentTarget.duration) && e.currentTarget.duration > 0) {
                          setVideoDuration(e.currentTarget.duration);
                        }
                      }}
                      onTimeUpdate={(e) => setVideoCurrentTime(e.currentTarget.currentTime)}
                      className="w-full h-full object-cover"
                    />

                    {/* Static First Frame Comparison Overlay */}
                    {compareVideoFrame && (
                      <>
                        <img
                          src="/mcp-demo/floating-garden.webp"
                          alt="首帧原图"
                          style={{ clipPath: "inset(0 50% 0 0)" }}
                          className="pointer-events-none absolute inset-0 w-full h-full object-cover"
                        />
                        <div className="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 w-[2px] bg-white/90 shadow-[0_0_10px_rgba(0,0,0,0.6)]" />
                        <span className="pointer-events-none absolute top-2.5 left-2.5 px-2 py-0.5 rounded-full bg-black/65 backdrop-blur-md text-white text-[10.5px] font-medium border border-white/15">
                          首帧原图 · 静态
                        </span>
                        <span className="pointer-events-none absolute top-2.5 right-2.5 px-2 py-0.5 rounded-full bg-[var(--accent)]/90 backdrop-blur-md text-white text-[10.5px] font-medium">
                          动态成片 · 流动中
                        </span>
                      </>
                    )}

                    {/* Floating Frosted Bottom Control Bar */}
                    <div className="absolute inset-x-2.5 bottom-2.5 px-3 py-2 rounded-xl bg-black/55 backdrop-blur-md border border-white/15 flex flex-wrap sm:flex-nowrap items-center gap-2.5 text-white">
                      <button
                        type="button"
                        data-testid="mcp-demo-play-video"
                        onClick={() => void handleToggleVideo()}
                        aria-label={videoPlaying ? "暂停视频" : "播放视频"}
                        className="h-7 w-7 rounded-full bg-white text-slate-900 hover:bg-white/90 flex items-center justify-center shrink-0 transition cursor-pointer"
                      >
                        {videoPlaying ? (
                          <Pause className="h-3.5 w-3.5" />
                        ) : (
                          <Play className="h-3.5 w-3.5 ml-0.5" />
                        )}
                      </button>

                      {/* Scrubbable Progress Bar */}
                      <div
                        role="slider"
                        aria-label="视频播放进度"
                        aria-valuemin={0}
                        aria-valuemax={videoDur}
                        aria-valuenow={Number(videoCurrentTime.toFixed(1))}
                        onPointerDown={(e) => {
                          e.currentTarget.setPointerCapture(e.pointerId);
                          handleScrubVideo(e.clientX, e.currentTarget.getBoundingClientRect());
                        }}
                        onPointerMove={(e) => {
                          if (e.buttons === 1) {
                            handleScrubVideo(e.clientX, e.currentTarget.getBoundingClientRect());
                          }
                        }}
                        className="flex-1 min-w-[90px] py-2 cursor-pointer flex items-center"
                      >
                        <div className="w-full h-1.5 rounded-full bg-white/25 overflow-hidden">
                          <div
                            className="h-full rounded-full bg-white transition-[width] duration-75"
                            style={{ width: `${videoProgressPct}%` }}
                          />
                        </div>
                      </div>

                      <span className="text-[11px] font-mono tabular-nums text-white/90 shrink-0">
                        {videoCurrentTime.toFixed(1)}s / {videoDur.toFixed(1)}s
                      </span>

                      <button
                        type="button"
                        data-testid="mcp-video-compare-frame"
                        onClick={() => setCompareVideoFrame((v) => !v)}
                        className={`px-2 py-1 rounded-lg text-[10.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer shrink-0 border ${
                          compareVideoFrame
                            ? "bg-[var(--accent)] text-white border-white/25"
                            : "bg-white/12 hover:bg-white/20 text-white border-white/15"
                        }`}
                      >
                        <SplitSquareHorizontal className="h-3 w-3" />
                        <span>对比首帧原图</span>
                      </button>
                    </div>
                  </div>
                </div>

                {/* Single-line capability pills */}
                <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-[var(--text-secondary)]">
                  {["首尾帧插值", "@image0 角色锁定", "MP4 尾帧续写", "超时凭任务号续传免重扣"].map(
                    (cap) => (
                      <span
                        key={cap}
                        className="px-2 py-0.5 rounded-full bg-[var(--bg-card)]/75 border border-[var(--border)]"
                      >
                        {cap}
                      </span>
                    ),
                  )}
                </div>
              </div>
            )}

            {/* 3. AUDIO CANVAS */}
            {activeTab === "audio" && (
              <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-card)]/80 backdrop-blur-sm p-3 sm:p-3.5 flex flex-col gap-3 min-w-0">
                <audio
                  ref={audioRef}
                  src={currentTrack.src}
                  preload="metadata"
                  onLoadedMetadata={(e) => {
                    if (
                      Number.isFinite(e.currentTarget.duration) &&
                      e.currentTarget.duration > 0
                    ) {
                      setAudioDuration(e.currentTarget.duration);
                    }
                  }}
                  onTimeUpdate={(e) => setAudioCurrentTime(e.currentTarget.currentTime)}
                  onEnded={() => {
                    setAudioPlaying(false);
                    setAudioCurrentTime(0);
                  }}
                />

                {/* 3 Emotion Track Capsules */}
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex flex-wrap items-center gap-1.5">
                    {AUDIO_TRACKS.map((track) => {
                      const active = audioEmotion === track.key;
                      return (
                        <button
                          key={track.key}
                          type="button"
                          data-testid={`mcp-audio-emotion-${track.key}`}
                          onClick={() => setAudioEmotion(track.key)}
                          className={`px-2.5 py-1 rounded-full text-[11.5px] font-medium border transition cursor-pointer ${
                            active
                              ? "bg-[var(--accent)] text-white border-transparent shadow-sm"
                              : "bg-[var(--bg-canvas)] text-[var(--text-secondary)] border-[var(--border)] hover:text-[var(--text-primary)]"
                          }`}
                        >
                          {track.label}
                        </button>
                      );
                    })}
                  </div>

                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-[var(--text-muted)]">
                    <Volume2 className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span>{currentTrack.badge}</span>
                  </span>
                </div>

                {/* Play Button + 28 Dynamic Rhythm Bars */}
                <div className="flex items-center gap-3 min-w-0">
                  <button
                    type="button"
                    data-testid="mcp-demo-play-audio"
                    onClick={() => void handleToggleAudio()}
                    aria-label={audioPlaying ? "暂停冰糖配音" : "播放冰糖配音"}
                    className="h-10 w-10 rounded-full bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white flex items-center justify-center shrink-0 transition cursor-pointer shadow-sm"
                  >
                    {audioPlaying ? (
                      <Pause className="h-4 w-4" />
                    ) : (
                      <Play className="h-4 w-4 ml-0.5" />
                    )}
                  </button>

                  <div
                    role="slider"
                    aria-label="音频波形进度条"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(audioProgressRatio * 100)}
                    onClick={(e) => {
                      const rect = e.currentTarget.getBoundingClientRect();
                      const ratio = (e.clientX - rect.left) / Math.max(1, rect.width);
                      void handleJumpAudioRatio(ratio);
                    }}
                    className="flex-1 min-w-0 h-11 flex items-end gap-[2px] sm:gap-[3px] cursor-pointer px-1 py-1"
                  >
                    {WAVEFORM_28.map((heightPct, idx) => {
                      const barRatio = (idx + 0.5) / WAVEFORM_28.length;
                      const isPassed = barRatio <= audioProgressRatio;
                      return (
                        <span
                          key={idx}
                          style={{
                            height: `${heightPct}%`,
                            animationDelay: `${(idx % 7) * 85}ms`,
                            animationDuration: `${580 + (idx % 5) * 90}ms`,
                          }}
                          className={`flex-1 rounded-full transition-colors duration-150 ${
                            audioPlaying ? "mcp-wave-bar-active" : ""
                          } ${
                            isPassed
                              ? "bg-[var(--accent)]"
                              : audioPlaying
                                ? "bg-[var(--accent)]/45"
                                : "bg-[var(--border-strong)]/80"
                          }`}
                        />
                      );
                    })}
                  </div>

                  <span className="tabular-nums font-mono text-[11px] text-[var(--text-secondary)] shrink-0">
                    {formatAudioTime(audioCurrentTime)} / {formatAudioTime(audioDuration)}
                  </span>
                </div>

                {/* Synchronized Phrase-by-Phrase Clickable Subtitles */}
                <div
                  data-testid="mcp-audio-subtitles"
                  className="flex flex-wrap items-center gap-1.5 leading-relaxed pt-1 border-t border-[var(--border)]/70"
                >
                  {currentTrack.phrases.map((phrase, idx) => {
                    const isActive = activePhraseIdx === idx;
                    return (
                      <button
                        key={idx}
                        type="button"
                        onClick={() => void handleJumpAudioRatio(phraseRanges[idx]?.start ?? 0)}
                        className={`text-left px-2 py-1 rounded-lg text-[12px] transition-all duration-200 cursor-pointer ${
                          isActive
                            ? "bg-[var(--accent-soft)] text-[var(--accent)] font-semibold scale-[1.01] shadow-[inset_0_0_0_1px_var(--accent-border)]"
                            : "text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]/70"
                        }`}
                      >
                        {phrase}
                      </button>
                    );
                  })}
                </div>

                {audioError && (
                  <div className="text-[11px] text-[var(--danger)]">{audioError}</div>
                )}
              </div>
            )}

            {/* 4. SEARCH CANVAS */}
            {activeTab === "search" && (
              <div className="flex flex-col gap-2.5 min-w-0">
                {/* Typewriter Streaming Summary with Interactive [1][2][3] Badges */}
                <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-card)]/85 px-3.5 py-3 text-[12.5px] leading-relaxed text-[var(--text-primary)] selectable">
                  {renderSearchSummary()}
                </div>

                {/* Real Source Frosted Cards */}
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 min-w-0">
                  {searchScene.citations.map((item, idx) => {
                    const highlighted = activeCitationIdx === idx;
                    return (
                      <button
                        key={item.url}
                        type="button"
                        onMouseEnter={() => setActiveCitationIdx(idx)}
                        onMouseLeave={() => setActiveCitationIdx(null)}
                        onClick={() => void api.openUrl(item.url)}
                        className={`text-left rounded-xl border p-2.5 flex flex-col gap-1 transition-all cursor-pointer min-w-0 backdrop-blur-sm ${
                          highlighted
                            ? "border-[var(--accent)] bg-[var(--accent-soft)]/70 shadow-sm -translate-y-0.5"
                            : "border-[var(--border)] bg-[var(--bg-card)]/75 hover:border-[var(--accent-border)] hover:bg-[var(--bg-card)]"
                        }`}
                      >
                        <div className="flex items-center justify-between gap-1.5 min-w-0">
                          <span className="inline-flex items-center gap-1 text-[10.5px] font-semibold text-[var(--accent)] truncate">
                            <span className="px-1 rounded bg-[var(--accent-soft)]">
                              [{idx + 1}]
                            </span>
                            <span className="truncate">{item.domain}</span>
                          </span>
                          <ExternalLink className="h-3 w-3 text-[var(--text-muted)] shrink-0" />
                        </div>
                        <div className="text-[11.5px] font-semibold text-[var(--text-primary)] line-clamp-1">
                          {item.title}
                        </div>
                        <div className="text-[11px] text-[var(--text-secondary)] line-clamp-2 leading-snug">
                          {item.snippet}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}

            {/* 5. CONSULT CANVAS */}
            {activeTab === "consult" && (
              <div className="flex flex-col gap-2.5 min-w-0">
                {/* Dual-Model Sparring Header */}
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 min-w-0">
                  <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-card)]/80 px-3 py-2 flex flex-col gap-1 min-w-0">
                    <span className="text-[10.5px] font-semibold text-[var(--text-muted)]">
                      主模型发起会诊
                    </span>
                    <span className="text-[11.5px] text-[var(--text-primary)] leading-snug">
                      {consultScene.question}
                    </span>
                  </div>

                  <div className="rounded-xl border border-[var(--accent-border)] bg-[var(--accent-soft)]/70 px-3 py-2 flex flex-col justify-between gap-1 min-w-0">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[10.5px] font-semibold text-[var(--accent)] truncate">
                        外脑参谋 ({consultModel}) 独立审阅
                      </span>
                      <ShieldCheck className="h-3.5 w-3.5 text-[var(--accent)] shrink-0" />
                    </div>
                    <span className="text-[11.5px] text-[var(--text-primary)] leading-snug">
                      已隔离主上下文偏见，输出 3 项结构化盲点拆解与可直接落地的改造方案。
                    </span>
                  </div>
                </div>

                {/* 3 Structured Diagnostic Micro-Cards */}
                <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 min-w-0">
                  {consultScene.cards.map((card, idx) => {
                    const visible = idx < consultRevealCount;
                    const tagColor =
                      card.tone === "danger"
                        ? "text-[var(--danger)] bg-[var(--danger-soft)]"
                        : card.tone === "success"
                          ? "text-[var(--success)] bg-[var(--success-soft)]"
                          : "text-[var(--warning)] bg-[var(--warning-soft)]";
                    return (
                      <div
                        key={card.tag}
                        className={`rounded-xl border border-[var(--border)] bg-[var(--bg-card)]/85 p-2.5 flex flex-col gap-1 transition-all duration-300 min-w-0 ${
                          visible ? "opacity-100 translate-y-0" : "opacity-0 translate-y-1.5"
                        }`}
                      >
                        <div className="flex items-center justify-between gap-1.5">
                          <span
                            className={`px-1.5 py-0.5 rounded-md text-[10px] font-semibold ${tagColor}`}
                          >
                            {card.tag}
                          </span>
                        </div>
                        <div className="text-[11.5px] font-semibold text-[var(--text-primary)]">
                          {card.title}
                        </div>
                        <div className="text-[11px] text-[var(--text-secondary)] leading-snug">
                          {card.detail}
                        </div>
                      </div>
                    );
                  })}
                </div>

                {/* Embedded Quick Consult Model Switcher */}
                <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-card)]/75 px-3 py-2">
                  <span className="text-[11px] font-medium text-[var(--text-secondary)]">
                    外脑参谋默认模型
                  </span>
                  <div className="flex flex-wrap items-center gap-1.5">
                    {CONSULT_MODEL_PRESETS.map((m) => {
                      const active = consultModel === m;
                      return (
                        <button
                          key={m}
                          type="button"
                          data-testid={`consult-preset-${m}`}
                          disabled={!mcp || savingMcp}
                          onClick={() => onSelectConsultModel(m)}
                          className={`px-2 py-0.5 rounded-full text-[11px] font-mono border transition cursor-pointer disabled:opacity-60 ${
                            active
                              ? "bg-[var(--accent)] text-white border-transparent font-semibold"
                              : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
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

          {/* Theater Footer Bar */}
          <div className="pt-1 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border)]/70">
            <button
              type="button"
              data-testid="mcp-demo-copy-prompt"
              onClick={() => void handleCopyPrompt()}
              className="h-7 px-2.5 rounded-full border border-[var(--border)] bg-[var(--bg-card)]/80 hover:bg-[var(--bg-card)] text-[11.5px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer"
            >
              {copiedPrompt ? (
                <>
                  <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                  <span className="text-[var(--success)]">已复制对话指令</span>
                </>
              ) : (
                <>
                  <Copy className="h-3.5 w-3.5" />
                  <span>复制对话指令</span>
                </>
              )}
            </button>

            <button
              type="button"
              data-testid="mcp-jump-live"
              onClick={handleJumpLive}
              className="h-7 px-3 rounded-full bg-[var(--accent-soft)] hover:bg-[var(--accent)] text-[var(--accent)] hover:text-white border border-[var(--accent-border)] text-[11.5px] font-semibold inline-flex items-center gap-1 transition cursor-pointer"
            >
              <Zap className="h-3 w-3" />
              <span>⚡ 改词现场试跑</span>
            </button>
          </div>
        </div>
      ) : (
        /* Mode B: Live Run Mode */
        <div className="flex flex-col gap-2.5 min-w-0">
          {!loggedIn ? (
            <div className="rounded-2xl border border-[var(--accent-border)] bg-[var(--bg-card)]/90 p-5 flex flex-col items-center text-center gap-3">
              <div className="h-10 w-10 rounded-full bg-[var(--accent-soft)] text-[var(--accent)] flex items-center justify-center">
                <Zap className="h-5 w-5" />
              </div>
              <div className="flex flex-col gap-1 max-w-[360px]">
                <div className="text-[13px] font-semibold text-[var(--text-primary)]">
                  登录 Tako 账号后即可在此直接调用内置网关现场生成
                </div>
                <div className="text-[11.5px] text-[var(--text-secondary)]">
                  支持生图、生视频、冰糖配音、实时联网溯源与外脑会诊，生成结果即时在画布内预览。
                </div>
              </div>
              <button
                type="button"
                data-testid="mcp-live-login-btn"
                disabled={loggingIn}
                onClick={onRequestLogin}
                className="h-8 px-4 rounded-full bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold transition cursor-pointer disabled:opacity-60"
              >
                {loggingIn ? "登录中..." : "立即登录"}
              </button>
            </div>
          ) : (
            <>
              {/* Editable Prompt Box */}
              <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-card)]/90 p-2.5 flex flex-col gap-2 min-w-0">
                <textarea
                  data-testid="mcp-live-prompt-input"
                  rows={2}
                  value={livePrompts[activeTab]}
                  onChange={(e) =>
                    setLivePrompts((prev) => ({
                      ...prev,
                      [activeTab]: e.target.value,
                    }))
                  }
                  placeholder={`输入自然语言提示词，现场体验${currentMeta.label}...`}
                  className="w-full resize-none bg-transparent text-[12.5px] text-[var(--text-primary)] placeholder:text-[var(--text-muted)] focus:outline-none leading-relaxed"
                />

                {/* Tool-Specific Lightweight Parameter Pills + Run Button */}
                <div className="flex flex-wrap items-center justify-between gap-2 pt-2 border-t border-[var(--border)]/70">
                  <div className="flex flex-wrap items-center gap-1.5 min-w-0">
                    {activeTab === "image" && (
                      <>
                        <div className="inline-flex flex-wrap items-center gap-1">
                          {IMAGE_MODELS.map((m) => (
                            <button
                              key={m}
                              type="button"
                              onClick={() => setLiveImageModel(m)}
                              className={`px-2 py-0.5 rounded-full text-[10.5px] font-mono border transition cursor-pointer ${
                                liveImageModel === m
                                  ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-semibold"
                                  : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                              }`}
                            >
                              {m}
                            </button>
                          ))}
                        </div>
                        <div className="inline-flex items-center p-0.5 rounded-full bg-[var(--bg-pill)] border border-[var(--border)]">
                          {(["1:1", "16:9"] as const).map((ratio) => (
                            <button
                              key={ratio}
                              type="button"
                              onClick={() => setLiveAspectRatio(ratio)}
                              className={`px-2 py-0.5 rounded-full text-[10.5px] font-mono transition cursor-pointer ${
                                liveAspectRatio === ratio
                                  ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-semibold shadow-xs"
                                  : "text-[var(--text-secondary)]"
                              }`}
                            >
                              {ratio}
                            </button>
                          ))}
                        </div>
                      </>
                    )}

                    {activeTab === "video" && (
                      <span className="px-2.5 py-0.5 rounded-full bg-[var(--bg-canvas)] border border-[var(--border)] text-[11px] text-[var(--text-secondary)]">
                        grok-imagine-video-1.5 · 5s 480p（按秒计费，超时可免重扣取回）
                      </span>
                    )}

                    {activeTab === "audio" && (
                      <>
                        <div className="inline-flex items-center gap-1">
                          {AUDIO_VOICES.map((v) => (
                            <button
                              key={v}
                              type="button"
                              onClick={() => setLiveAudioVoice(v)}
                              className={`px-2 py-0.5 rounded-full text-[11px] border transition cursor-pointer ${
                                liveAudioVoice === v
                                  ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-semibold"
                                  : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)]"
                              }`}
                            >
                              {v}
                            </button>
                          ))}
                        </div>
                        <div className="inline-flex items-center gap-1">
                          {AUDIO_TONES.map((t) => (
                            <button
                              key={t}
                              type="button"
                              onClick={() => setLiveAudioTone(t)}
                              className={`px-2 py-0.5 rounded-full text-[10.5px] border transition cursor-pointer ${
                                liveAudioTone === t
                                  ? "bg-[var(--bg-card)] border-[var(--border-strong)] text-[var(--text-primary)] font-medium"
                                  : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-muted)]"
                              }`}
                            >
                              {t}
                            </button>
                          ))}
                        </div>
                      </>
                    )}

                    {activeTab === "search" && (
                      <div className="inline-flex items-center gap-1">
                        {SEARCH_ENGINES.map((engine) => (
                          <button
                            key={engine}
                            type="button"
                            onClick={() => setLiveSearchEngine(engine)}
                            className={`px-2.5 py-0.5 rounded-full text-[11px] font-mono border transition cursor-pointer ${
                              liveSearchEngine === engine
                                ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-semibold"
                                : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)]"
                            }`}
                          >
                            {engine}
                          </button>
                        ))}
                      </div>
                    )}

                    {activeTab === "consult" && (
                      <div className="inline-flex flex-wrap items-center gap-1">
                        {CONSULT_MODEL_PRESETS.map((m) => (
                          <button
                            key={m}
                            type="button"
                            data-testid={`consult-preset-${m}`}
                            disabled={!mcp || savingMcp}
                            onClick={() => onSelectConsultModel(m)}
                            className={`px-2 py-0.5 rounded-full text-[10.5px] font-mono border transition cursor-pointer disabled:opacity-60 ${
                              consultModel === m
                                ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-semibold"
                                : "bg-[var(--bg-canvas)] border-[var(--border)] text-[var(--text-secondary)]"
                            }`}
                          >
                            {m}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  <button
                    type="button"
                    data-testid="mcp-live-run-btn"
                    disabled={runningLive || !livePrompts[activeTab].trim()}
                    onClick={() => void handleRunLiveDemo()}
                    className="h-7 px-3.5 rounded-full bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11.5px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-60 shrink-0"
                  >
                    {runningLive ? (
                      <>
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        <span className="tabular-nums">
                          生成中 {(liveElapsedMs / 1000).toFixed(1)}s...
                        </span>
                      </>
                    ) : (
                      <>
                        <Zap className="h-3.5 w-3.5" />
                        <span>立即生成</span>
                      </>
                    )}
                  </button>
                </div>
              </div>

              {liveError && (
                <div className="rounded-xl border border-[var(--danger)]/30 bg-[var(--danger-soft)] px-3 py-2 text-[11.5px] text-[var(--danger)]">
                  试跑失败：{liveError}
                </div>
              )}

              {/* Live Run Result Canvas */}
              {currentLiveResult && (
                <div
                  data-testid="mcp-live-result"
                  className="rounded-2xl border border-[var(--border)] bg-[var(--bg-card)]/90 p-3 flex flex-col gap-2.5 min-w-0"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2 text-[11px]">
                    <div className="flex items-center gap-2 min-w-0">
                      <span
                        className={`px-2 py-0.5 rounded-full font-medium ${
                          currentLiveResult.status === "pending"
                            ? "bg-[var(--warning-soft)] text-[var(--warning)]"
                            : "bg-[var(--success-soft)] text-[var(--success)]"
                        }`}
                      >
                        {currentLiveResult.status === "pending" ? "云端渲染中" : "已生成"}
                      </span>
                      <span className="font-mono text-[var(--text-secondary)] truncate">
                        {currentLiveResult.model}
                      </span>
                    </div>
                    <span className="tabular-nums font-mono text-[var(--text-muted)]">
                      {(currentLiveResult.elapsedMs / 1000).toFixed(2)}s
                    </span>
                  </div>

                  {/* Image Result */}
                  {currentLiveResult.tool === "image" && currentLiveResult.previewDataUrl && (
                    <div className="rounded-xl overflow-hidden border border-[var(--border)] bg-black/5 flex justify-center">
                      <img
                        src={currentLiveResult.previewDataUrl}
                        alt="现场生图结果"
                        className="max-h-[340px] w-auto object-contain"
                      />
                    </div>
                  )}

                  {/* Video Result */}
                  {currentLiveResult.tool === "video" && (
                    <div className="flex flex-col gap-2">
                      {currentLiveResult.previewDataUrl && (
                        <video
                          controls
                          autoPlay
                          loop
                          playsInline
                          src={currentLiveResult.previewDataUrl}
                          className="w-full max-h-[320px] rounded-xl bg-black border border-[var(--border)]"
                        />
                      )}
                      {currentLiveResult.status === "pending" && currentLiveResult.taskId && (
                        <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--warning)]/35 bg-[var(--warning-soft)] px-3 py-2">
                          <span className="text-[11.5px] text-[var(--warning)] font-medium">
                            任务仍在云端生成中（任务号 {currentLiveResult.taskId}）
                          </span>
                          <button
                            type="button"
                            data-testid="mcp-live-resume-btn"
                            disabled={runningLive}
                            onClick={() => void handleRunLiveDemo(currentLiveResult.taskId ?? undefined)}
                            className="h-7 px-3 rounded-full bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11px] font-semibold transition cursor-pointer disabled:opacity-60"
                          >
                            继续轮询取回视频（免重复扣费）
                          </button>
                        </div>
                      )}
                    </div>
                  )}

                  {/* Audio Result */}
                  {currentLiveResult.tool === "audio" && currentLiveResult.previewDataUrl && (
                    <audio
                      controls
                      autoPlay
                      src={currentLiveResult.previewDataUrl}
                      className="w-full"
                    />
                  )}

                  {/* Search Result */}
                  {currentLiveResult.tool === "search" && (
                    <div className="flex flex-col gap-2 min-w-0">
                      {currentLiveResult.textOutput && (
                        <div className="text-[12px] text-[var(--text-primary)] leading-relaxed whitespace-pre-wrap selectable">
                          {currentLiveResult.textOutput}
                        </div>
                      )}
                      {currentLiveResult.citations.length > 0 && (
                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-1.5">
                          {currentLiveResult.citations.map((c, i) => (
                            <button
                              key={`${c.url}-${i}`}
                              type="button"
                              onClick={() => void api.openUrl(c.url)}
                              className="text-left rounded-xl border border-[var(--border)] bg-[var(--bg-canvas)] hover:border-[var(--accent-border)] p-2 flex flex-col gap-0.5 transition cursor-pointer min-w-0"
                            >
                              <div className="flex items-center justify-between gap-1 text-[10.5px] font-semibold text-[var(--accent)]">
                                <span className="truncate">{c.domain}</span>
                                <ExternalLink className="h-3 w-3 shrink-0" />
                              </div>
                              <div className="text-[11.5px] font-medium text-[var(--text-primary)] truncate">
                                {c.title}
                              </div>
                              {c.snippet && (
                                <div className="text-[10.5px] text-[var(--text-secondary)] line-clamp-2">
                                  {c.snippet}
                                </div>
                              )}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  {/* Consult Result */}
                  {currentLiveResult.tool === "consult" && currentLiveResult.textOutput && (
                    <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-canvas)] p-3 text-[12px] text-[var(--text-primary)] leading-relaxed whitespace-pre-wrap selectable">
                      {currentLiveResult.textOutput}
                    </div>
                  )}

                  {/* Saved Path Pill */}
                  {currentLiveResult.savedPath && (
                    <div className="flex items-center justify-between gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-canvas)] px-2.5 py-1.5 text-[11px] min-w-0">
                      <code className="font-mono text-[var(--text-secondary)] truncate">
                        已保存至：{currentLiveResult.savedPath}
                      </code>
                      <button
                        type="button"
                        onClick={() => void handleCopySavedPath(currentLiveResult.savedPath!)}
                        className="px-2 py-0.5 rounded-md border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[10.5px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1 shrink-0 cursor-pointer"
                      >
                        {copiedSavedPath ? (
                          <>
                            <Check className="h-3 w-3 text-[var(--success)]" />
                            <span>已复制</span>
                          </>
                        ) : (
                          <>
                            <Copy className="h-3 w-3" />
                            <span>复制路径</span>
                          </>
                        )}
                      </button>
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
