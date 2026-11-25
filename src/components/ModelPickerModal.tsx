import { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  Brain,
  Check,
  CheckCheck,
  CheckCircle2,
  Coins,
  Copy,
  Flame,
  Gauge,
  Info,
  Layers,
  LayoutGrid,
  Lock,
  RefreshCw,
  RotateCcw,
  RotateCw,
  Search,
  Sparkles,
  Star,
  X,
} from "lucide-react";
import { AppBrandIcon } from "./BrandIcons";
import { ModelFamilyIcon } from "./ModelIcons";
import {
  api,
  type AppId,
  type AppSnapshot,
  type ClientModelFilter,
  type ClientModelOption,
  type ClientModelsResponse,
  type ModelFilterMode,
  type SwitchState,
} from "../lib/api";
import {
  formatOfficialDiscountBadge,
  formatPlazaMoney,
  formatPromoBadge,
  matchesPlazaQuickFilter,
  readStoredPlazaCurrency,
  readStoredPlazaPriceMode,
  readStoredTokenUnit,
  writeStoredPlazaCurrency,
  writeStoredPlazaPriceMode,
  writeStoredTokenUnit,
  type ModelCapability,
  type PlazaCurrency,
  type PlazaPriceMode,
  type PlazaQuickFilterId,
  type TokenUnit,
} from "../lib/plaza-pricing";

interface ModelPickerModalProps {
  appId: AppId | null;
  appName: string;
  appRunning: boolean;
  loggedIn?: boolean;
  apps?: AppSnapshot[];
  onClose: () => void;
  onSaved: (appId: AppId, nextState: SwitchState, restarted: boolean) => void;
}

type CategoryFilter = "__all__" | "__selected__" | string;
type ModelSortBy = "default" | "price" | "speed";

const VIEW_TABS: { id: AppId; label: string; shortLabel: string }[] = [
  { id: "dsh-tako", label: "广场总览", shortLabel: "广场" },
  { id: "codex", label: "Codex", shortLabel: "Codex" },
  { id: "claude", label: "Claude", shortLabel: "Claude" },
  { id: "antigravity", label: "Antigravity", shortLabel: "反重力" },
];

const QUICK_FILTERS: {
  id: PlazaQuickFilterId;
  label: string;
  testId: string;
}[] = [
  { id: "all", label: "全部", testId: "plaza-quick-all" },
  { id: "promo", label: "🔥 限时特惠", testId: "plaza-quick-promo" },
  { id: "cheap", label: "≤9折", testId: "plaza-quick-cheap" },
  { id: "code", label: "编程", testId: "plaza-quick-code" },
  { id: "long", label: "1M+上下文", testId: "plaza-quick-long" },
  { id: "image", label: "生图", testId: "plaza-quick-image" },
];

const CAPABILITY_FILTERS: { id: ModelCapability; label: string }[] = [
  { id: "chat", label: "对话" },
  { id: "reason", label: "推理" },
  { id: "vision", label: "视觉" },
  { id: "code", label: "编程" },
  { id: "image", label: "生图" },
  { id: "video", label: "视频" },
  { id: "audio", label: "语音" },
  { id: "embed", label: "向量" },
];

const CAPABILITY_LABELS: Record<string, string> = {
  chat: "对话",
  reason: "推理",
  vision: "视觉",
  code: "编程",
  image: "生图",
  video: "视频",
  audio: "语音",
  embed: "向量",
};

const CLIENT_HINTS: Record<AppId, string> = {
  "dsh-tako":
    "免登录实时同步网关 /api/pricing 全量目录；切换上方客户端标签可勾选并下发专属模型列表。",
  claude:
    "Claude Desktop 仅展示 Claude 系列模型：勾选并保存后会自动写入本地 Claude 3p 配置。",
  codex: "Codex 原生支持多厂商模型，可切换精简推荐或自由勾选展示列表。",
  antigravity:
    "Antigravity 最多映射 32 个自定义槽位，精简勾选可避免主力模型被裁掉。",
};

function formatContextWindow(tokens: number | null): string | null {
  if (!tokens || tokens <= 0) return null;
  if (tokens >= 1_000_000) {
    const m = tokens / 1_000_000;
    return Number.isInteger(m) ? `${m}M` : `${Number(m.toFixed(2))}M`;
  }
  if (tokens >= 1_000) {
    return `${Math.round(tokens / 1_000)}K`;
  }
  return `${tokens}`;
}

function getPrimaryAggregatedPrice(
  m: ClientModelOption,
): { usd: number; source: "realized" | "standard" | "input" } | null {
  if (m.realizedUsdPerM != null && m.realizedUsdPerM > 0) {
    return { usd: m.realizedUsdPerM, source: "realized" };
  }
  if (m.standardLoadUsdPerM != null && m.standardLoadUsdPerM > 0) {
    return { usd: m.standardLoadUsdPerM, source: "standard" };
  }
  if (m.inputUsdPerM != null && m.inputUsdPerM > 0) {
    return { usd: m.inputUsdPerM, source: "input" };
  }
  return null;
}

function getEffectiveSortPrice(m: ClientModelOption, priceMode: PlazaPriceMode): number | null {
  if (m.isFree) return 0;
  if (m.quotaType === 1 || (m.requestUsd != null && m.inputUsdPerM == null)) {
    return m.requestUsd != null && m.requestUsd > 0 ? m.requestUsd : null;
  }
  if (priceMode === "detail") {
    if (m.inputUsdPerM != null && m.inputUsdPerM > 0) return m.inputUsdPerM;
  }
  const primary = getPrimaryAggregatedPrice(m);
  if (primary) return primary.usd;
  if (m.requestUsd != null && m.requestUsd > 0) return m.requestUsd;
  return null;
}

function getSpeedBadge(
  m: ClientModelOption,
): {
  label: string;
  style: { backgroundColor: string; color: string };
} | null {
  if (m.avgTps == null || m.avgTps <= 0) return null;
  const label =
    m.avgTps >= 10 ? `${Math.round(m.avgTps)} t/s` : `${m.avgTps.toFixed(1)} t/s`;
  if (m.avgTps >= 150) {
    return {
      label,
      style: { backgroundColor: "var(--success-soft)", color: "var(--success)" },
    };
  }
  if (m.avgTps >= 60) {
    return {
      label,
      style: { backgroundColor: "var(--accent-soft)", color: "var(--accent)" },
    };
  }
  return {
    label,
    style: { backgroundColor: "var(--warning-soft)", color: "var(--warning)" },
  };
}

function buildCardTitle(
  m: ClientModelOption,
  currency: PlazaCurrency,
  tokenUnit: TokenUnit,
  lockedLast: boolean,
): string {
  const parts: string[] = [`ID: ${m.id}`];
  if (m.vendorName) parts.push(`厂商: ${m.vendorName}`);
  if (m.description) parts.push(m.description);

  const discount = formatOfficialDiscountBadge(m.officialMultiplier);
  if (discount) parts.push(`官方折算: ${discount}`);

  const promo = formatPromoBadge(m.promoEndsOn);
  if (promo) parts.push(`活动: ${promo}`);

  if (m.isFree) {
    parts.push("价格: 免费");
  } else if (m.quotaType === 1 || (m.requestUsd != null && m.inputUsdPerM == null)) {
    const reqText = formatPlazaMoney(m.requestUsd, {
      currency,
      tokenUnit,
      perToken: false,
    });
    if (reqText) parts.push(`按次计费: ${reqText}/次`);
  } else {
    const inText = formatPlazaMoney(m.inputUsdPerM, { currency, tokenUnit });
    const cacheText = formatPlazaMoney(m.cacheUsdPerM, { currency, tokenUnit });
    const outText = formatPlazaMoney(m.outputUsdPerM, { currency, tokenUnit });
    if (inText || outText) {
      parts.push(
        `明细单价 (/${tokenUnit}): 输入 ${inText ?? "—"} · 缓存读 ${cacheText ?? "—"} · 输出 ${outText ?? "—"}`,
      );
    }
    if (m.realizedUsdPerM != null && m.realizedUsdPerM > 0) {
      const realText = formatPlazaMoney(m.realizedUsdPerM, { currency, tokenUnit });
      parts.push(`近7日实付均价: ${realText}/${tokenUnit}`);
    }
    if (m.standardLoadUsdPerM != null && m.standardLoadUsdPerM > 0) {
      const stdText = formatPlazaMoney(m.standardLoadUsdPerM, { currency, tokenUnit });
      parts.push(`标准负载均价: ${stdText}/${tokenUnit}`);
    }
    if (m.cacheHitRate != null) {
      parts.push(`近7日缓存命中率: ${(m.cacheHitRate * 100).toFixed(1)}%`);
    }
    if (m.channels.length > 0) {
      const chText = m.channels
        .slice(0, 4)
        .map((c) => {
          const chLoad = formatPlazaMoney(c.standardLoadUsdPerM, { currency, tokenUnit });
          return chLoad ? `${c.name} (${chLoad}/${tokenUnit})` : `${c.name} (${c.ratio}x)`;
        })
        .join(" · ");
      parts.push(`可用渠道: ${chText}`);
    }
  }

  if (m.avgTps != null || m.avgLatencyMs != null) {
    const perfSegs: string[] = [];
    if (m.avgTps != null && m.avgTps > 0) perfSegs.push(`吞吐 ${m.avgTps.toFixed(1)} tok/s`);
    if (m.avgLatencyMs != null && m.avgLatencyMs > 0) {
      perfSegs.push(`平均耗时 ${(m.avgLatencyMs / 1000).toFixed(1)}s`);
    }
    if (m.successRate != null) perfSegs.push(`成功率 ${m.successRate.toFixed(1)}%`);
    if (perfSegs.length > 0) parts.push(perfSegs.join(" · "));
  }

  if (lockedLast) {
    parts.push("（至少保留 1 个展示模型）");
  }
  return parts.join("\n");
}

interface ModelCardProps {
  model: ClientModelOption;
  isPlazaOverview: boolean;
  checked: boolean;
  lockedLast: boolean;
  isRecommended: boolean;
  copied: boolean;
  saving: boolean;
  currency: PlazaCurrency;
  tokenUnit: TokenUnit;
  priceMode: PlazaPriceMode;
  onSelect: (modelId: string) => void;
}

const ModelCard = memo(function ModelCard({
  model,
  isPlazaOverview,
  checked,
  lockedLast,
  isRecommended,
  copied,
  saving,
  currency,
  tokenUnit,
  priceMode,
  onSelect,
}: ModelCardProps) {
  const ctxLabel = formatContextWindow(model.contextWindow);
  const speedBadge = getSpeedBadge(model);
  const discountBadge = formatOfficialDiscountBadge(model.officialMultiplier);
  const promoBadge = formatPromoBadge(model.promoEndsOn);
  const isPerRequest =
    model.quotaType === 1 || (model.requestUsd != null && model.inputUsdPerM == null);

  const inPrice = formatPlazaMoney(model.inputUsdPerM, { currency, tokenUnit });
  const cachePrice = formatPlazaMoney(model.cacheUsdPerM, { currency, tokenUnit });
  const outPrice = formatPlazaMoney(model.outputUsdPerM, { currency, tokenUnit });
  const inOrigPromo =
    model.promoEndsOn && model.inputUsdPerM != null
      ? formatPlazaMoney(model.inputUsdPerM * 2, { currency, tokenUnit })
      : null;
  const outOrigPromo =
    model.promoEndsOn && model.outputUsdPerM != null
      ? formatPlazaMoney(model.outputUsdPerM * 2, { currency, tokenUnit })
      : null;

  const primaryAvg = getPrimaryAggregatedPrice(model);
  const avgMainText = primaryAvg
    ? formatPlazaMoney(primaryAvg.usd, { currency, tokenUnit })
    : null;
  const stdCompareText =
    model.standardLoadUsdPerM != null && model.standardLoadUsdPerM > 0
      ? formatPlazaMoney(model.standardLoadUsdPerM, { currency, tokenUnit })
      : null;
  const reqPriceText = formatPlazaMoney(model.requestUsd, {
    currency,
    tokenUnit,
    perToken: false,
  });

  const titleText = useMemo(
    () => buildCardTitle(model, currency, tokenUnit, lockedLast),
    [currency, lockedLast, model, tokenUnit],
  );

  return (
    <button
      type="button"
      disabled={saving}
      onClick={() => onSelect(model.id)}
      title={titleText}
      className={`group text-left tako-mcp-cell rounded-[11px] border transition flex flex-col gap-1.5 min-w-0 cursor-pointer ${
        !isPlazaOverview && checked
          ? "border-[var(--accent-border)] bg-[var(--accent-soft)]/35 hover:bg-[var(--accent-soft)]/55"
          : "border-[var(--border)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]/40"
      }`}
    >
      {/* Row 1: Icon + Name + Check/Copy */}
      <div className="flex items-start justify-between gap-2 min-w-0">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <ModelFamilyIcon family={model.family} size="sm" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5 min-w-0">
              <span className="text-[12.5px] font-semibold text-[var(--text-primary)] truncate">
                {model.displayName}
              </span>
              {isRecommended && (
                <Star className="h-3 w-3 text-[var(--warning)] fill-current shrink-0" />
              )}
            </div>
            <div className="text-[10.5px] font-mono text-[var(--text-muted)] truncate">
              {model.id}
            </div>
          </div>
        </div>

        {isPlazaOverview ? (
          <span
            className="h-5 px-1.5 rounded bg-[var(--bg-subtle)] text-[var(--text-muted)] group-hover:text-[var(--text-primary)] inline-flex items-center gap-1 text-[10px] shrink-0"
            title="点击复制模型 ID"
          >
            {copied ? (
              <>
                <Check className="h-2.5 w-2.5 text-[var(--success)]" />
                <span className="text-[var(--success)]">已复制</span>
              </>
            ) : (
              <>
                <Copy className="h-2.5 w-2.5" />
                <span>复制</span>
              </>
            )}
          </span>
        ) : (
          <span
            className={`h-4 w-4 rounded-[5px] flex items-center justify-center shrink-0 transition ${
              checked
                ? "bg-[var(--accent)] text-white"
                : "border border-[var(--border-strong)] bg-[var(--bg-card)]"
            }`}
          >
            {lockedLast ? (
              <Lock className="h-2.5 w-2.5" />
            ) : checked ? (
              <Check className="h-3 w-3 stroke-[2.5]" />
            ) : null}
          </span>
        )}
      </div>

      {/* Row 2: Promo & Official Multiplier Badges */}
      {(promoBadge || discountBadge || model.isDynamicPricing) && (
        <div className="flex flex-wrap items-center gap-1">
          {promoBadge && (
            <span
              className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-semibold"
              style={{
                backgroundColor: "var(--warning-soft)",
                color: "var(--warning)",
              }}
            >
              <Flame className="h-2.5 w-2.5 shrink-0" />
              <span>{promoBadge.replace(/^🔥\s*/, "")}</span>
            </span>
          )}
          {discountBadge && (
            <span
              className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium tabular-nums"
              style={
                (model.officialMultiplier ?? 1) <= 1.005
                  ? {
                      backgroundColor: "var(--success-soft)",
                      color: "var(--success)",
                    }
                  : {
                      backgroundColor: "var(--bg-subtle)",
                      color: "var(--text-secondary)",
                    }
              }
            >
              {discountBadge}
            </span>
          )}
          {model.isDynamicPricing && (
            <span className="inline-flex items-center px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-secondary)] text-[10px]">
              阶梯计费
            </span>
          )}
        </div>
      )}

      {/* Row 3: PR #296 Pricing Block (Detail vs Avg) */}
      <div className="rounded-[8px] bg-[var(--bg-canvas)]/75 border border-[var(--border)] px-2 py-1.5 text-[11px]">
        {model.isFree ? (
          <div className="flex items-center justify-between">
            <span className="text-[var(--text-muted)]">平台计费</span>
            <span
              className="font-semibold px-1.5 py-0.5 rounded text-[10.5px]"
              style={{
                backgroundColor: "var(--success-soft)",
                color: "var(--success)",
              }}
            >
              免费可用
            </span>
          </div>
        ) : isPerRequest && reqPriceText ? (
          <div className="flex items-center justify-between tabular-nums">
            <span className="text-[var(--text-muted)]">按次计费</span>
            <span className="font-semibold text-[var(--text-primary)]">
              {reqPriceText} <span className="text-[10px] font-normal text-[var(--text-muted)]">/ 次</span>
            </span>
          </div>
        ) : priceMode === "detail" ? (
          <div className="grid grid-cols-3 gap-1.5 tabular-nums">
            <div className="min-w-0">
              <div className="text-[10px] text-[var(--text-muted)]">输入/{tokenUnit}</div>
              <div className="font-semibold text-[var(--text-primary)] truncate">
                {inPrice ?? "—"}
                {inOrigPromo && (
                  <span className="ml-1 text-[10px] font-normal line-through text-[var(--text-muted)]">
                    {inOrigPromo}
                  </span>
                )}
              </div>
            </div>
            <div className="min-w-0">
              <div className="text-[10px] text-[var(--text-muted)]">缓存读/{tokenUnit}</div>
              <div className="font-semibold text-[var(--success)] truncate">
                {cachePrice ?? "—"}
              </div>
            </div>
            <div className="min-w-0">
              <div className="text-[10px] text-[var(--text-muted)]">输出/{tokenUnit}</div>
              <div className="font-semibold text-[var(--text-primary)] truncate">
                {outPrice ?? "—"}
                {outOrigPromo && (
                  <span className="ml-1 text-[10px] font-normal line-through text-[var(--text-muted)]">
                    {outOrigPromo}
                  </span>
                )}
              </div>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-0.5 tabular-nums">
            <div className="flex items-center justify-between gap-1">
              <span className="text-[10.5px] text-[var(--text-muted)]">
                {primaryAvg?.source === "realized"
                  ? "7日实付均价"
                  : primaryAvg?.source === "standard"
                    ? "标准负载均价"
                    : "输入基准价"}
              </span>
              <span className="text-[12px] font-semibold text-[var(--accent)]">
                {avgMainText ? `${avgMainText}/${tokenUnit}` : "—"}
              </span>
            </div>
            <div className="flex items-center justify-between gap-1 text-[10px] text-[var(--text-muted)]">
              <span className="truncate">
                标准负载 {stdCompareText ? `${stdCompareText}/${tokenUnit}` : "—"}
              </span>
              <span>
                {model.cacheHitRate != null
                  ? `缓存命中 ${(model.cacheHitRate * 100).toFixed(0)}%`
                  : "无近期缓存统计"}
              </span>
            </div>
          </div>
        )}
      </div>

      {/* Row 4: Description & Meta Pills */}
      <div className="flex items-center justify-between gap-1.5 text-[10.5px] text-[var(--text-secondary)] min-w-0 mt-auto">
        <span className="truncate flex-1">{model.description}</span>
        <div className="flex items-center gap-1 shrink-0">
          {speedBadge && (
            <span
              className="px-1.5 py-0.5 rounded text-[10px] font-medium tabular-nums"
              style={speedBadge.style}
            >
              {speedBadge.label}
            </span>
          )}
          {ctxLabel && (
            <span className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-muted)] text-[10px] tabular-nums">
              <Layers className="h-2.5 w-2.5" />
              <span>{ctxLabel}</span>
            </span>
          )}
          {model.effortLevels.length > 0 && (
            <span className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-muted)] text-[10px] tabular-nums">
              <Brain className="h-2.5 w-2.5" />
              <span>{model.effortLevels.length}档</span>
            </span>
          )}
          {!ctxLabel &&
            model.capabilities
              .filter((c) => c !== "chat")
              .slice(0, 2)
              .map((cap) => (
                <span
                  key={cap}
                  className="px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-muted)] text-[10px]"
                >
                  {CAPABILITY_LABELS[cap] ?? cap}
                </span>
              ))}
        </div>
      </div>
    </button>
  );
});

export function ModelPickerModal({
  appId,
  appName,
  appRunning,
  loggedIn = true,
  apps,
  onClose,
  onSaved,
}: ModelPickerModalProps) {
  const [activeView, setActiveView] = useState<AppId>(appId ?? "dsh-tako");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<ClientModelsResponse | null>(null);

  const [mode, setMode] = useState<ModelFilterMode>("recommended");
  const [customIds, setCustomIds] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [activeCategory, setActiveCategory] = useState<CategoryFilter>("__all__");
  const [quickFilter, setQuickFilter] = useState<PlazaQuickFilterId>("all");
  const [capabilityFilter, setCapabilityFilter] = useState<ModelCapability | null>(null);
  const [sortBy, setSortBy] = useState<ModelSortBy>("default");
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const [currency, setCurrency] = useState<PlazaCurrency>(() => readStoredPlazaCurrency());
  const [tokenUnit, setTokenUnit] = useState<TokenUnit>(() => readStoredTokenUnit());
  const [priceMode, setPriceMode] = useState<PlazaPriceMode>(() => readStoredPlazaPriceMode());

  const cacheRef = useRef<Partial<Record<AppId, ClientModelsResponse>>>({});
  const inflightRef = useRef<Partial<Record<AppId, Promise<ClientModelsResponse>>>>({});
  const reqSeqRef = useRef(0);
  const prefetchStartedRef = useRef(false);

  useEffect(() => {
    if (appId) {
      setActiveView(appId);
    }
  }, [appId]);

  const isPlazaOverview = activeView === "dsh-tako";

  const activeClientRunning = useMemo(() => {
    if (isPlazaOverview) return false;
    const snap = apps?.find((a) => a.id === activeView);
    if (snap) return snap.running;
    return activeView === appId ? appRunning : false;
  }, [activeView, appId, appRunning, apps, isPlazaOverview]);

  const handleSelectCurrency = (next: PlazaCurrency) => {
    setCurrency(next);
    writeStoredPlazaCurrency(next);
  };

  const handleSelectUnit = (next: TokenUnit) => {
    setTokenUnit(next);
    writeStoredTokenUnit(next);
  };

  const handleSelectPriceMode = (next: PlazaPriceMode) => {
    setPriceMode(next);
    writeStoredPlazaPriceMode(next);
  };

  const applyResponse = useCallback((resp: ClientModelsResponse) => {
    setData(resp);
    setMode(resp.filter.mode);
    if (resp.filter.mode === "custom" && resp.filter.enabledIds.length > 0) {
      setCustomIds(new Set(resp.filter.enabledIds.map((s) => s.toLowerCase())));
    } else {
      const defaults = resp.models.filter((m) => m.visible).map((m) => m.id.toLowerCase());
      setCustomIds(new Set(defaults));
    }
  }, []);

  const fetchView = useCallback(
    (view: AppId, forceRefresh: boolean): Promise<ClientModelsResponse> => {
      if (!forceRefresh) {
        const pending = inflightRef.current[view];
        if (pending) return pending;
      }
      const req =
        view === "dsh-tako"
          ? api.listPlazaModels(forceRefresh)
          : api.listClientModels(view, forceRefresh);
      inflightRef.current[view] = req;
      return req.finally(() => {
        if (inflightRef.current[view] === req) {
          delete inflightRef.current[view];
        }
      });
    },
    [],
  );

  const prefetchOtherViews = useCallback(
    (currentView: AppId) => {
      if (prefetchStartedRef.current) return;
      prefetchStartedRef.current = true;
      const targets: AppId[] = loggedIn
        ? (["dsh-tako", "codex", "claude", "antigravity"] as AppId[]).filter(
            (v) => v !== currentView && !cacheRef.current[v],
          )
        : currentView !== "dsh-tako" && !cacheRef.current["dsh-tako"]
          ? ["dsh-tako"]
          : [];
      for (const view of targets) {
        void fetchView(view, false)
          .then((resp) => {
            if (!cacheRef.current[view]) {
              cacheRef.current[view] = resp;
            }
          })
          .catch(() => {
            // Silent best-effort warm-up; errors only surface if the user opens that tab.
          });
      }
    },
    [fetchView, loggedIn],
  );

  const loadModels = useCallback(
    async (targetView: AppId, forceRefresh = false) => {
      const seq = ++reqSeqRef.current;
      if (forceRefresh) {
        cacheRef.current = {};
        prefetchStartedRef.current = false;
      } else {
        const cached = cacheRef.current[targetView];
        if (cached) {
          setError(null);
          setLoading(false);
          applyResponse(cached);
          if (appId === "dsh-tako") {
            prefetchOtherViews(targetView);
          }
          return;
        }
        setData(null);
      }

      setLoading(true);
      setError(null);
      try {
        const resp = await fetchView(targetView, forceRefresh);
        cacheRef.current[targetView] = resp;
        if (seq !== reqSeqRef.current) return;
        applyResponse(resp);
        if (appId === "dsh-tako") {
          prefetchOtherViews(targetView);
        }
      } catch (err) {
        if (seq !== reqSeqRef.current) return;
        if (!forceRefresh) {
          setData(null);
        }
        setError(String(err));
      } finally {
        if (seq === reqSeqRef.current) {
          setLoading(false);
        }
      }
    },
    [appId, applyResponse, fetchView, prefetchOtherViews],
  );

  useEffect(() => {
    if (!appId) {
      reqSeqRef.current += 1;
      cacheRef.current = {};
      inflightRef.current = {};
      prefetchStartedRef.current = false;
      setLoading(false);
      setData(null);
      setError(null);
      setSearch("");
      setActiveCategory("__all__");
      setQuickFilter("all");
      setCapabilityFilter(null);
      setSortBy("default");
      return;
    }
  }, [appId]);

  useEffect(() => {
    if (!appId) return;
    if (appId !== "dsh-tako" && activeView !== appId) return;
    setActiveCategory("__all__");
    void loadModels(activeView);
  }, [appId, activeView, loadModels]);

  useEffect(() => {
    if (!appId) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !saving) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [appId, onClose, saving]);

  const recommendedSet = useMemo(() => {
    if (!data) return new Set<string>();
    const s = new Set(data.recommendedIds.map((id) => id.toLowerCase()));
    for (const m of data.models) {
      if (m.recommended) s.add(m.id.toLowerCase());
    }
    return s;
  }, [data]);

  const checkedIds = useMemo(() => {
    if (!data) return new Set<string>();
    if (isPlazaOverview) {
      return new Set(data.models.map((m) => m.id.toLowerCase()));
    }
    if (mode === "all") {
      return new Set(data.models.map((m) => m.id.toLowerCase()));
    }
    if (mode === "recommended") {
      const matched = data.models
        .filter((m) => recommendedSet.has(m.id.toLowerCase()))
        .map((m) => m.id.toLowerCase());
      if (matched.length > 0) return new Set(matched);
      return new Set(data.models.slice(0, 3).map((m) => m.id.toLowerCase()));
    }
    return customIds;
  }, [customIds, data, isPlazaOverview, mode, recommendedSet]);

  const quickFilterCounts = useMemo(() => {
    const counts: Record<PlazaQuickFilterId, number> = {
      all: 0,
      promo: 0,
      cheap: 0,
      code: 0,
      long: 0,
      image: 0,
    };
    if (!data) return counts;
    for (const m of data.models) {
      for (const q of QUICK_FILTERS) {
        if (matchesPlazaQuickFilter(m, q.id)) {
          counts[q.id] += 1;
        }
      }
    }
    return counts;
  }, [data]);

  const familyStats = useMemo(() => {
    if (!data) return [];
    const map = new Map<string, { family: string; total: number; checked: number }>();
    for (const m of data.models) {
      const cur = map.get(m.family) ?? { family: m.family, total: 0, checked: 0 };
      cur.total += 1;
      if (checkedIds.has(m.id.toLowerCase())) cur.checked += 1;
      map.set(m.family, cur);
    }
    return Array.from(map.values());
  }, [checkedIds, data]);

  const filteredModels = useMemo(() => {
    if (!data) return [];
    const q = deferredSearch.trim().toLowerCase();
    const matched = data.models.filter((m) => {
      if (!matchesPlazaQuickFilter(m, quickFilter)) {
        return false;
      }
      if (capabilityFilter && !(m.capabilities ?? []).includes(capabilityFilter)) {
        return false;
      }
      if (activeCategory === "__selected__") {
        if (!checkedIds.has(m.id.toLowerCase())) return false;
      } else if (activeCategory !== "__all__" && m.family !== activeCategory) {
        return false;
      }
      if (!q) return true;
      return (
        m.id.toLowerCase().includes(q) ||
        m.displayName.toLowerCase().includes(q) ||
        m.family.toLowerCase().includes(q) ||
        (m.vendorName ?? "").toLowerCase().includes(q) ||
        m.description.toLowerCase().includes(q) ||
        (m.tags ?? []).some((t) => t.toLowerCase().includes(q))
      );
    });

    if (sortBy === "default") return matched;

    const indexed = matched.map((item, idx) => ({ item, idx }));
    if (sortBy === "price") {
      indexed.sort((a, b) => {
        const pa = getEffectiveSortPrice(a.item, priceMode);
        const pb = getEffectiveSortPrice(b.item, priceMode);
        if (pa == null && pb == null) return a.idx - b.idx;
        if (pa == null) return 1;
        if (pb == null) return -1;
        if (pa !== pb) return pa - pb;
        return a.idx - b.idx;
      });
    } else if (sortBy === "speed") {
      indexed.sort((a, b) => {
        const sa = a.item.avgTps != null && a.item.avgTps > 0 ? a.item.avgTps : null;
        const sb = b.item.avgTps != null && b.item.avgTps > 0 ? b.item.avgTps : null;
        if (sa == null && sb == null) return a.idx - b.idx;
        if (sa == null) return 1;
        if (sb == null) return -1;
        if (sb !== sa) return sb - sa;
        return a.idx - b.idx;
      });
    }
    return indexed.map((entry) => entry.item);
  }, [
    activeCategory,
    capabilityFilter,
    checkedIds,
    data,
    deferredSearch,
    priceMode,
    quickFilter,
    sortBy,
  ]);

  const handleSelectMode = (nextMode: ModelFilterMode) => {
    if (!data || isPlazaOverview) return;
    if (nextMode === "custom" && mode !== "custom") {
      setCustomIds(new Set(checkedIds));
    }
    setMode(nextMode);
  };

  const selectionStateRef = useRef({
    isPlazaOverview,
    mode,
    customIds,
    checkedIds,
  });
  selectionStateRef.current = {
    isPlazaOverview,
    mode,
    customIds,
    checkedIds,
  };

  const handleCardClick = useCallback((modelId: string) => {
    const {
      isPlazaOverview: plaza,
      mode: curMode,
      customIds: curCustom,
      checkedIds: curChecked,
    } = selectionStateRef.current;
    if (plaza) {
      try {
        void navigator.clipboard?.writeText(modelId);
        setCopiedId(modelId);
        window.setTimeout(() => {
          setCopiedId((prev) => (prev === modelId ? null : prev));
        }, 1400);
      } catch {
        // ignore clipboard failure
      }
      return;
    }
    const key = modelId.toLowerCase();
    const baseSet = curMode === "custom" ? new Set(curCustom) : new Set(curChecked);
    if (baseSet.has(key)) {
      if (baseSet.size <= 1) return;
      baseSet.delete(key);
    } else {
      baseSet.add(key);
    }
    setMode("custom");
    setCustomIds(baseSet);
  }, []);

  const visibleUnselectedCount = useMemo(() => {
    if (isPlazaOverview) return 0;
    let count = 0;
    for (const m of filteredModels) {
      if (!checkedIds.has(m.id.toLowerCase())) count += 1;
    }
    return count;
  }, [checkedIds, filteredModels, isPlazaOverview]);

  const handleBatchToggleFiltered = () => {
    if (!data || isPlazaOverview || filteredModels.length === 0) return;
    const baseSet = mode === "custom" ? new Set(customIds) : new Set(checkedIds);
    if (visibleUnselectedCount > 0) {
      for (const m of filteredModels) {
        baseSet.add(m.id.toLowerCase());
      }
    } else {
      for (const m of filteredModels) {
        if (baseSet.size <= 1) break;
        baseSet.delete(m.id.toLowerCase());
      }
    }
    setMode("custom");
    setCustomIds(baseSet);
  };

  const handleResetDefault = () => {
    if (isPlazaOverview) {
      setQuickFilter("all");
      setCapabilityFilter(null);
      setActiveCategory("__all__");
      setSearch("");
      setSortBy("default");
      return;
    }
    setMode("recommended");
    setError(null);
  };

  const handleSave = async (restartNow: boolean) => {
    if (!data || isPlazaOverview) return;
    setSaving(true);
    setError(null);
    try {
      const orderedIds =
        mode === "custom"
          ? data.models.filter((m) => customIds.has(m.id.toLowerCase())).map((m) => m.id)
          : [];
      const payload: ClientModelFilter = {
        mode,
        enabledIds: orderedIds,
      };
      const nextState = await api.saveClientModelFilter(activeView, payload);
      delete cacheRef.current[activeView];
      let restarted = false;
      if (restartNow) {
        await api.restartApp(activeView);
        restarted = true;
      }
      onSaved(activeView, nextState, restarted);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  if (!appId) return null;

  const isAuthMissingError =
    !isPlazaOverview &&
    Boolean(
      error &&
        (error.includes("未登录") || error.includes("缺少") || error.includes("API Key")),
    );

  return (
    <div
      className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-2 sm:p-3.5"
      onClick={() => {
        if (!saving) onClose();
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-[860px] h-[min(660px,92vh)] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] flex flex-col overflow-hidden"
      >
        {/* Top Header: Context-focused client title (or Plaza tabs) + Mode + Actions */}
        <div className="px-4 py-3 border-b border-[var(--border)] flex flex-wrap items-center justify-between gap-2.5 shrink-0 bg-[var(--bg-card)]">
          <div className="flex items-center flex-wrap gap-2 min-w-0">
            {appId === "dsh-tako" ? (
              <div className="inline-flex items-center flex-wrap p-0.5 rounded-[10px] bg-[var(--bg-pill)] border border-[var(--border)]/80 max-w-full">
                {VIEW_TABS.map((tab) => {
                  const active = activeView === tab.id;
                  return (
                    <button
                      key={tab.id}
                      type="button"
                      disabled={saving}
                      onClick={() => setActiveView(tab.id)}
                      className={`h-7 px-2.5 rounded-[8px] text-[12px] inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-50 ${
                        active
                          ? "bg-[var(--bg-card)] text-[var(--accent)] font-semibold shadow-[var(--shadow-segment)]"
                          : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      <span className="hidden sm:inline-flex items-center shrink-0">
                        {tab.id === "dsh-tako" ? (
                          <Sparkles className="h-3.5 w-3.5 text-[var(--accent)]" />
                        ) : (
                          <AppBrandIcon appId={tab.id} className="h-4 w-4 rounded-[4px]" />
                        )}
                      </span>
                      <span className="sm:hidden">{tab.shortLabel}</span>
                      <span className="hidden sm:inline">{tab.label}</span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="flex items-center gap-2 min-w-0">
                <AppBrandIcon appId={activeView} className="h-5 w-5" />
                <span className="text-[14px] font-semibold text-[var(--text-primary)] truncate">
                  {appName || activeView} · 模型列表
                </span>
              </div>
            )}

            {data && (
              <span className="tabular-nums text-[11px] font-medium px-2 py-0.5 rounded-full bg-[var(--accent-soft)] text-[var(--accent)] shrink-0">
                {isPlazaOverview
                  ? `共 ${data.totalCount} 款`
                  : `已选 ${checkedIds.size}/${data.totalCount}`}
              </span>
            )}
          </div>

          <div className="flex items-center gap-1.5 shrink-0 ml-auto">
            {!isPlazaOverview && (
              <div className="inline-flex p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)]">
                {(
                  [
                    { id: "recommended", label: "推荐" },
                    { id: "custom", label: "自选" },
                    { id: "all", label: "全部" },
                  ] as const
                ).map((opt) => {
                  const active = mode === opt.id;
                  return (
                    <button
                      key={opt.id}
                      type="button"
                      disabled={!data || loading || saving}
                      onClick={() => handleSelectMode(opt.id)}
                      className={`h-6 px-2 rounded-[7px] text-[11.5px] transition cursor-pointer disabled:opacity-50 ${
                        active
                          ? "bg-[var(--bg-card)] text-[var(--accent)] font-medium shadow-[var(--shadow-segment)]"
                          : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      {opt.label}
                    </button>
                  );
                })}
              </div>
            )}

            <button
              type="button"
              disabled={loading || saving}
              onClick={() => void loadModels(activeView, true)}
              title="重新同步网关最新模型与实付价格"
              className="h-7 w-7 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] flex items-center justify-center transition cursor-pointer disabled:opacity-50"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
            </button>

            <button
              type="button"
              disabled={saving}
              onClick={onClose}
              className="h-7 w-7 rounded-[8px] text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] flex items-center justify-center cursor-pointer"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* PR #296 Pricing Toolbar + 6 Quick Scene Filters */}
        <div className="px-3.5 py-2 border-b border-[var(--border)] bg-[var(--bg-canvas)]/45 flex flex-col gap-2 shrink-0">
          <div className="flex flex-wrap items-center justify-between gap-1.5">
            {/* 6 Quick Filter Capsules */}
            <div className="flex items-center flex-wrap gap-1 min-w-0">
              {QUICK_FILTERS.map((qf) => {
                const active = quickFilter === qf.id;
                const count = quickFilterCounts[qf.id];
                return (
                  <button
                    key={qf.id}
                    type="button"
                    data-testid={`plaza-quick-${qf.id}`}
                    onClick={() => setQuickFilter(qf.id)}
                    className={`h-6 px-2 rounded-full text-[11px] font-medium inline-flex items-center gap-1 transition cursor-pointer border ${
                      active
                        ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)]"
                        : "bg-[var(--bg-card)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]"
                    }`}
                  >
                    <span>{qf.label}</span>
                    {data && qf.id !== "all" && count > 0 && (
                      <span className="tabular-nums text-[10px] opacity-80">{count}</span>
                    )}
                  </button>
                );
              })}
            </div>

            {/* Pricing Controls: [明细价 | 均价] + [¥ 人民币 | $ 额度] + [/1M | /1K] */}
            <div className="flex items-center flex-wrap gap-1.5 shrink-0">
              <div className="inline-flex p-0.5 rounded-[8px] bg-[var(--bg-pill)] border border-[var(--border)]">
                <button
                  type="button"
                  data-testid="plaza-price-mode-detail"
                  onClick={() => handleSelectPriceMode("detail")}
                  className={`h-6 px-2 rounded-[6px] text-[11px] transition cursor-pointer ${
                    priceMode === "detail"
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  明细价
                </button>
                <button
                  type="button"
                  data-testid="plaza-price-mode-avg"
                  onClick={() => handleSelectPriceMode("avg")}
                  className={`h-6 px-2 rounded-[6px] text-[11px] transition cursor-pointer ${
                    priceMode === "avg"
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  均价
                </button>
              </div>

              <div className="inline-flex p-0.5 rounded-[8px] bg-[var(--bg-pill)] border border-[var(--border)]">
                <button
                  type="button"
                  data-testid="plaza-currency-cny"
                  onClick={() => handleSelectCurrency("cny")}
                  className={`h-6 px-2 rounded-[6px] text-[11px] transition cursor-pointer ${
                    currency === "cny"
                      ? "bg-[var(--bg-card)] text-[var(--accent)] font-semibold shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  ¥ 人民币
                </button>
                <button
                  type="button"
                  data-testid="plaza-currency-usd"
                  onClick={() => handleSelectCurrency("internal-usd")}
                  className={`h-6 px-2 rounded-[6px] text-[11px] transition cursor-pointer ${
                    currency === "internal-usd"
                      ? "bg-[var(--bg-card)] text-[var(--warning)] font-semibold shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  $ 额度
                </button>
              </div>

              <div className="inline-flex p-0.5 rounded-[8px] bg-[var(--bg-pill)] border border-[var(--border)]">
                <button
                  type="button"
                  data-testid="plaza-unit-m"
                  onClick={() => handleSelectUnit("M")}
                  className={`h-6 px-1.5 rounded-[6px] text-[11px] tabular-nums transition cursor-pointer ${
                    tokenUnit === "M"
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  /1M
                </button>
                <button
                  type="button"
                  data-testid="plaza-unit-k"
                  onClick={() => handleSelectUnit("K")}
                  className={`h-6 px-1.5 rounded-[6px] text-[11px] tabular-nums transition cursor-pointer ${
                    tokenUnit === "K"
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                      : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  /1K
                </button>
              </div>
            </div>
          </div>

          {currency === "internal-usd" && (
            <div
              className="px-2.5 py-1 rounded-[8px] text-[11px] flex items-center gap-1.5"
              style={{
                backgroundColor: "var(--warning-soft)",
                color: "var(--warning)",
              }}
            >
              <Info className="h-3.5 w-3.5 shrink-0" />
              <span>按充值 ¥1 = $3 折算，非美元现金（默认推荐使用 ¥ 人民币实付价对比）</span>
            </div>
          )}
        </div>

        {/* Error Banner */}
        {error && !isAuthMissingError && (
          <div
            className="mx-3.5 mt-2.5 rounded-[10px] px-3 py-2 text-[12px] flex items-center justify-between gap-2 border shrink-0"
            style={{
              backgroundColor: "var(--danger-soft)",
              borderColor: "rgba(220, 38, 38, 0.24)",
              color: "var(--danger)",
            }}
          >
            <span className="inline-flex items-center gap-1.5 min-w-0">
              <AlertCircle className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{error}</span>
            </span>
            <button
              type="button"
              onClick={() => void loadModels(activeView)}
              className="px-2 py-0.5 rounded bg-[var(--bg-card)] text-[11px] font-medium shrink-0 cursor-pointer"
            >
              重试
            </button>
          </div>
        )}

        {/* Split Body: Left Vendor & Capability Sidebar (desktop) + Right Model Cards */}
        <div className="flex-1 min-h-0 flex overflow-hidden">
          <aside className="hidden sm:flex w-[166px] shrink-0 border-r border-[var(--border)] bg-[var(--bg-canvas)]/55 p-2 overflow-y-auto subtle-scroll flex-col gap-0.5">
            <button
              type="button"
              onClick={() => setActiveCategory("__all__")}
              className={`w-full px-2 py-1.5 rounded-[8px] flex items-center justify-between gap-1.5 text-[12px] transition cursor-pointer ${
                activeCategory === "__all__"
                  ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)] border border-[var(--border)]"
                  : "border border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]/70"
              }`}
            >
              <span className="inline-flex items-center gap-2 min-w-0">
                <span className="h-5 w-5 rounded-[6px] bg-[var(--bg-subtle)] text-[var(--text-secondary)] flex items-center justify-center shrink-0">
                  <LayoutGrid className="h-3 w-3" />
                </span>
                <span className="truncate">全部厂商</span>
              </span>
              {data && (
                <span className="tabular-nums text-[11px] text-[var(--text-muted)] shrink-0">
                  {data.totalCount}
                </span>
              )}
            </button>

            {!isPlazaOverview && (
              <button
                type="button"
                onClick={() => setActiveCategory("__selected__")}
                className={`w-full px-2 py-1.5 rounded-[8px] flex items-center justify-between gap-1.5 text-[12px] transition cursor-pointer ${
                  activeCategory === "__selected__"
                    ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)] border border-[var(--border)]"
                    : "border border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]/70"
                }`}
              >
                <span className="inline-flex items-center gap-2 min-w-0">
                  <span className="h-5 w-5 rounded-[6px] bg-[var(--accent-soft)] text-[var(--accent)] flex items-center justify-center shrink-0">
                    <CheckCircle2 className="h-3 w-3" />
                  </span>
                  <span className="truncate">仅看已选</span>
                </span>
                {data && (
                  <span className="tabular-nums text-[11px] font-medium text-[var(--accent)] shrink-0">
                    {checkedIds.size}
                  </span>
                )}
              </button>
            )}

            {/* 8 Capabilities Filter */}
            <div className="mt-1.5 mb-1 px-1.5 flex items-center justify-between">
              <span className="text-[10.5px] font-semibold text-[var(--text-muted)]">能力筛选</span>
              {capabilityFilter && (
                <button
                  type="button"
                  onClick={() => setCapabilityFilter(null)}
                  className="text-[10px] text-[var(--accent)] hover:underline cursor-pointer"
                >
                  重置
                </button>
              )}
            </div>
            <div className="grid grid-cols-2 gap-1 px-1 mb-1.5">
              {CAPABILITY_FILTERS.map((cap) => {
                const active = capabilityFilter === cap.id;
                return (
                  <button
                    key={cap.id}
                    type="button"
                    onClick={() => setCapabilityFilter(active ? null : cap.id)}
                    className={`h-6 rounded-[6px] text-[11px] transition cursor-pointer border ${
                      active
                        ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-medium"
                        : "bg-[var(--bg-card)]/70 border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    {cap.label}
                  </button>
                );
              })}
            </div>

            {familyStats.length > 0 && <div className="my-1 border-t border-[var(--border)]" />}

            {familyStats.map((stat) => {
              const active = activeCategory === stat.family;
              return (
                <button
                  key={stat.family}
                  type="button"
                  onClick={() => setActiveCategory(stat.family)}
                  className={`w-full px-2 py-1.5 rounded-[8px] flex items-center justify-between gap-1.5 text-[12px] transition cursor-pointer ${
                    active
                      ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)] border border-[var(--border)]"
                      : "border border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]/70"
                  }`}
                >
                  <span className="inline-flex items-center gap-2 min-w-0">
                    <ModelFamilyIcon family={stat.family} size="sm" />
                    <span className="truncate">{stat.family}</span>
                  </span>
                  <span
                    className={`tabular-nums text-[11px] shrink-0 ${
                      !isPlazaOverview && stat.checked > 0
                        ? "text-[var(--accent)] font-medium"
                        : "text-[var(--text-muted)]"
                    }`}
                  >
                    {isPlazaOverview ? stat.total : `${stat.checked}/${stat.total}`}
                  </span>
                </button>
              );
            })}
          </aside>

          {/* Right Content Pane */}
          <div className="flex-1 min-w-0 flex flex-col bg-[var(--bg-card)]">
            {/* Search + Sort Pill + Batch Action */}
            <div className="px-3 py-2 border-b border-[var(--border)] flex flex-wrap items-center gap-2 shrink-0">
              <div className="relative flex-1 min-w-[140px]">
                <Search className="h-3.5 w-3.5 text-[var(--text-muted)] absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="搜索模型名、ID、厂商或能力..."
                  className="w-full h-8 pl-8 pr-7 rounded-[9px] border border-[var(--border)] bg-[var(--bg-canvas)]/70 text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)] focus:bg-[var(--bg-card)]"
                />
                {search && (
                  <button
                    type="button"
                    onClick={() => setSearch("")}
                    className="p-0.5 rounded text-[var(--text-muted)] hover:text-[var(--text-primary)] absolute right-2 top-1/2 -translate-y-1/2 cursor-pointer"
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>

              <div className="h-8 inline-flex p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)] shrink-0">
                {(
                  [
                    { id: "default", label: "推荐", Icon: null },
                    { id: "price", label: "价格↑", Icon: Coins },
                    { id: "speed", label: "吞吐↓", Icon: Gauge },
                  ] as const
                ).map(({ id, label, Icon }) => {
                  const active = sortBy === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      disabled={!data}
                      onClick={() => setSortBy(id)}
                      className={`px-2 rounded-[7px] text-[11.5px] inline-flex items-center justify-center gap-1 transition cursor-pointer disabled:opacity-50 ${
                        active
                          ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                          : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      {Icon && <Icon className="h-3 w-3 shrink-0" />}
                      <span>{label}</span>
                    </button>
                  );
                })}
              </div>

              {!isPlazaOverview && filteredModels.length > 0 && (
                <button
                  type="button"
                  disabled={saving}
                  onClick={handleBatchToggleFiltered}
                  className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1 transition cursor-pointer shrink-0"
                >
                  <CheckCheck className="h-3.5 w-3.5 text-[var(--accent)]" />
                  <span>{visibleUnselectedCount > 0 ? "全选当前" : "取消当前"}</span>
                </button>
              )}
            </div>

            {/* Cards Grid */}
            <div className="flex-1 min-h-0 overflow-y-auto subtle-scroll p-3">
              {loading && !data ? (
                <div className="h-full flex items-center justify-center text-[13px] text-[var(--text-secondary)]">
                  正在拉取网关模型广场与实付定价...
                </div>
              ) : isAuthMissingError || (!isPlazaOverview && !loggedIn && !data) ? (
                <div className="h-full flex flex-col items-center justify-center gap-3 text-center px-6">
                  <div className="h-10 w-10 rounded-full bg-[var(--accent-soft)] text-[var(--accent)] flex items-center justify-center">
                    <Lock className="h-5 w-5" />
                  </div>
                  <div className="text-[13px] font-medium text-[var(--text-primary)]">
                    登录 Tako 账号后即可为 {VIEW_TABS.find((t) => t.id === activeView)?.label} 勾选并下发专属模型
                  </div>
                  <div className="text-[12px] text-[var(--text-secondary)] max-w-[380px]">
                    您仍可随时切换回「广场总览」，无需登录即可查看全部模型的人民币实付单价、官方折扣倍率与近 7 日吞吐表现。
                  </div>
                  <button
                    type="button"
                    onClick={() => setActiveView("dsh-tako")}
                    className="h-8 px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer"
                  >
                    查看全量广场价格（免登录）
                  </button>
                </div>
              ) : filteredModels.length === 0 ? (
                <div className="h-full flex flex-col items-center justify-center gap-2 text-[12.5px] text-[var(--text-muted)]">
                  <span>没有匹配的模型</span>
                  {(quickFilter !== "all" || capabilityFilter || search || activeCategory !== "__all__") && (
                    <button
                      type="button"
                      onClick={handleResetDefault}
                      className="px-2.5 py-1 rounded-md border border-[var(--border)] bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] text-[11.5px] cursor-pointer"
                    >
                      清空筛选条件
                    </button>
                  )}
                </div>
              ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 tako-list-stack">
                  {filteredModels.map((model) => {
                    const key = model.id.toLowerCase();
                    const checked = checkedIds.has(key);
                    const lockedLast = !isPlazaOverview && checked && checkedIds.size <= 1;
                    const isRecommended = recommendedSet.has(key) || model.recommended;
                    return (
                      <ModelCard
                        key={model.id}
                        model={model}
                        isPlazaOverview={isPlazaOverview}
                        checked={checked}
                        lockedLast={lockedLast}
                        isRecommended={isRecommended}
                        copied={copiedId === model.id}
                        saving={saving}
                        currency={currency}
                        tokenUnit={tokenUnit}
                        priceMode={priceMode}
                        onSelect={handleCardClick}
                      />
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="px-3.5 py-2.5 border-t border-[var(--border)] flex flex-wrap items-center justify-between gap-2 shrink-0 bg-[var(--bg-card)]">
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <span
              className="inline-flex items-center gap-1.5 text-[11px] text-[var(--text-muted)] min-w-0"
              title={CLIENT_HINTS[activeView]}
            >
              <Info className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{CLIENT_HINTS[activeView]}</span>
            </span>
            {!isPlazaOverview && (
              <button
                type="button"
                disabled={!data || saving}
                onClick={handleResetDefault}
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11.5px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer disabled:opacity-50 shrink-0"
              >
                <RotateCcw className="h-3 w-3" />
                <span>恢复默认</span>
              </button>
            )}
          </div>

          {isPlazaOverview ? (
            <div className="flex items-center flex-wrap gap-1.5 shrink-0">
              {(["codex", "claude", "antigravity"] as const).map((cid) => (
                <button
                  key={cid}
                  type="button"
                  onClick={() => setActiveView(cid)}
                  className="h-7 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-subtle)]/70 hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                >
                  配到 {cid === "codex" ? "Codex" : cid === "claude" ? "Claude" : "反重力"}
                </button>
              ))}
              <button
                type="button"
                onClick={onClose}
                className="h-8 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer"
              >
                完成
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-2 shrink-0">
              <button
                type="button"
                disabled={saving}
                onClick={onClose}
                className="h-8 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer disabled:opacity-50"
              >
                取消
              </button>
              <button
                type="button"
                disabled={!data || saving || checkedIds.size === 0}
                onClick={() => void handleSave(false)}
                className={`h-8 px-3 rounded-[9px] text-[12px] font-medium cursor-pointer disabled:opacity-50 ${
                  activeClientRunning
                    ? "border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)]"
                    : "bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white"
                }`}
              >
                {saving ? "保存中..." : "保存"}
              </button>
              {activeClientRunning && (
                <button
                  type="button"
                  disabled={!data || saving || checkedIds.size === 0}
                  onClick={() => void handleSave(true)}
                  className="h-8 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium inline-flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
                >
                  <RotateCw className="h-3.5 w-3.5" />
                  <span>{saving ? "处理中..." : "保存并重启"}</span>
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
