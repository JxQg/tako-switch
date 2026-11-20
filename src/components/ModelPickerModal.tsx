import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  Brain,
  Check,
  CheckCheck,
  CheckCircle2,
  Clock,
  Coins,
  Gauge,
  Info,
  Layers,
  LayoutGrid,
  Lock,
  RefreshCw,
  RotateCcw,
  RotateCw,
  Search,
  SlidersHorizontal,
  Sparkles,
  Star,
  X,
  Zap,
} from "lucide-react";
import { AppBrandIcon } from "./BrandIcons";
import { ModelFamilyIcon } from "./ModelIcons";
import {
  api,
  type AppId,
  type ClientModelFilter,
  type ClientModelOption,
  type ClientModelsResponse,
  type ModelFilterMode,
  type SwitchState,
} from "../lib/api";

interface ModelPickerModalProps {
  appId: AppId | null;
  appName: string;
  appRunning: boolean;
  onClose: () => void;
  onSaved: (appId: AppId, nextState: SwitchState, restarted: boolean) => void;
}

type CategoryFilter = "__all__" | "__selected__" | string;
type ModelSortBy = "default" | "price" | "speed";

const CLIENT_HINTS: Record<AppId, string> = {
  claude:
    "Claude Desktop 仅展示 Claude 系列模型：勾选并保存后会自动写入本地 Claude 3p 配置，关闭并重新打开 Claude Desktop 即可生效。",
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

function formatPlazaUsd(usd: number): string {
  if (usd < 10) return `$${usd.toFixed(3)}`;
  return `$${Number(usd.toFixed(2))}`;
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

function getEffectiveSortPrice(m: ClientModelOption): number | null {
  if (m.isFree) return 0;
  const primary = getPrimaryAggregatedPrice(m);
  if (primary) return primary.usd;
  if (m.requestUsd != null && m.requestUsd > 0) return m.requestUsd;
  return null;
}

function getPriceBadge(
  m: ClientModelOption,
): { label: string; style: { backgroundColor: string; color: string } } | null {
  if (m.isFree) {
    return {
      label: "免费",
      style: { backgroundColor: "var(--success-soft)", color: "var(--success)" },
    };
  }
  const primary = getPrimaryAggregatedPrice(m);
  if (primary) {
    const style =
      primary.usd <= 0.2
        ? { backgroundColor: "var(--success-soft)", color: "var(--success)" }
        : primary.usd <= 1.0
          ? { backgroundColor: "var(--bg-subtle)", color: "var(--text-secondary)" }
          : { backgroundColor: "var(--warning-soft)", color: "var(--warning)" };
    return { label: `${formatPlazaUsd(primary.usd)}/M`, style };
  }
  if (m.requestUsd != null && m.requestUsd > 0) {
    const style =
      m.requestUsd <= 0.05
        ? { backgroundColor: "var(--success-soft)", color: "var(--success)" }
        : m.requestUsd <= 0.2
          ? { backgroundColor: "var(--bg-subtle)", color: "var(--text-secondary)" }
          : { backgroundColor: "var(--warning-soft)", color: "var(--warning)" };
    return { label: `${formatPlazaUsd(m.requestUsd)}/次`, style };
  }
  return null;
}

function getSpeedBadge(
  m: ClientModelOption,
): {
  label: string;
  tier: "fast" | "medium" | "slow";
  style: { backgroundColor: string; color: string };
} | null {
  if (m.avgTps == null || m.avgTps <= 0) return null;
  const label =
    m.avgTps >= 10 ? `${Math.round(m.avgTps)} t/s` : `${m.avgTps.toFixed(1)} t/s`;
  if (m.avgTps >= 150) {
    return {
      label,
      tier: "fast",
      style: { backgroundColor: "var(--success-soft)", color: "var(--success)" },
    };
  }
  if (m.avgTps >= 60) {
    return {
      label,
      tier: "medium",
      style: { backgroundColor: "var(--accent-soft)", color: "var(--accent)" },
    };
  }
  return {
    label,
    tier: "slow",
    style: { backgroundColor: "var(--warning-soft)", color: "var(--warning)" },
  };
}

function formatListedIoLine(m: ClientModelOption): string | null {
  if (m.inputUsdPerM == null) return null;
  if (m.outputUsdPerM != null) {
    return `最低渠道挂牌: 输入 ${formatPlazaUsd(m.inputUsdPerM)} / 输出 ${formatPlazaUsd(m.outputUsdPerM)} / 1M`;
  }
  return `最低渠道挂牌: 输入 ${formatPlazaUsd(m.inputUsdPerM)} / 1M`;
}

function buildCardTitle(m: ClientModelOption, lockedLast: boolean): string {
  const parts: string[] = [`ID: ${m.id}`, m.description];
  if (m.isFree) {
    parts.push("价格: 免费");
  } else {
    if (m.realizedUsdPerM != null && m.realizedUsdPerM > 0) {
      const prefix = m.isDynamicPricing ? "分段计价 · " : "";
      parts.push(
        `${prefix}近7日均价: ${formatPlazaUsd(m.realizedUsdPerM)}/1M（实扣金额 ÷ 总 token，含缓存读写）`,
      );
      const secSegs: string[] = [];
      if (m.cacheHitRate != null) {
        secSegs.push(`7日缓存命中率 ${(m.cacheHitRate * 100).toFixed(1)}%`);
      }
      if (m.standardLoadUsdPerM != null) {
        let stdSeg = `标准负载估算 ${formatPlazaUsd(m.standardLoadUsdPerM)}/1M`;
        if (
          m.standardLoadMaxUsdPerM != null &&
          m.standardLoadMaxUsdPerM > m.standardLoadUsdPerM
        ) {
          stdSeg += ` (${formatPlazaUsd(m.standardLoadUsdPerM)}~${formatPlazaUsd(m.standardLoadMaxUsdPerM)}/1M)`;
        }
        secSegs.push(stdSeg);
      }
      if (secSegs.length > 0) {
        parts.push(secSegs.join(" · "));
      }
      const listedLine = formatListedIoLine(m);
      if (listedLine) parts.push(listedLine);
    } else if (m.isDynamicPricing) {
      const dynSegs: string[] = ["分段计价"];
      if (m.standardLoadUsdPerM != null) {
        dynSegs.push(`标准负载 ${formatPlazaUsd(m.standardLoadUsdPerM)}/1M`);
      }
      if (m.cacheHitRate != null) {
        dynSegs.push(`7日缓存命中率 ${(m.cacheHitRate * 100).toFixed(1)}%`);
      }
      parts.push(dynSegs.join(" · "));
    } else if (m.standardLoadUsdPerM != null) {
      let loadLine = `标准负载价: ${formatPlazaUsd(m.standardLoadUsdPerM)}/1M（92%缓存读+7%输入+1%输出）`;
      if (
        m.standardLoadMaxUsdPerM != null &&
        m.standardLoadMaxUsdPerM > m.standardLoadUsdPerM
      ) {
        loadLine += ` · 渠道区间 ${formatPlazaUsd(m.standardLoadUsdPerM)}~${formatPlazaUsd(m.standardLoadMaxUsdPerM)}/1M`;
      }
      parts.push(loadLine);
      if (m.cacheHitRate != null) {
        parts.push(`7日缓存命中率 ${(m.cacheHitRate * 100).toFixed(1)}%`);
      }
      const listedLine = formatListedIoLine(m);
      if (listedLine) parts.push(listedLine);
    } else if (m.requestUsd != null) {
      parts.push(`价格: ${formatPlazaUsd(m.requestUsd)}/次`);
    } else {
      const listedLine = formatListedIoLine(m);
      if (listedLine) parts.push(listedLine);
    }

    if (m.channels.length > 0) {
      const chText = m.channels
        .slice(0, 4)
        .map((c) =>
          c.standardLoadUsdPerM != null
            ? `${c.name} (${formatPlazaUsd(c.standardLoadUsdPerM)}/M)`
            : `${c.name} (${c.ratio}x)`,
        )
        .join(" · ");
      parts.push(`可用渠道: ${chText}`);
    }
  }

  if (m.avgTps != null || m.avgLatencyMs != null) {
    const perfSegs: string[] = [];
    if (m.avgTps != null && m.avgTps > 0) {
      perfSegs.push(`吞吐 ${m.avgTps.toFixed(1)} tok/s`);
    }
    if (m.avgLatencyMs != null && m.avgLatencyMs > 0) {
      perfSegs.push(`平均耗时 ${(m.avgLatencyMs / 1000).toFixed(1)}s`);
    }
    if (m.successRate != null) {
      perfSegs.push(`成功率 ${m.successRate.toFixed(1)}%`);
    }
    parts.push(perfSegs.join(" · "));
  }

  if (lockedLast) {
    parts.push("（至少保留 1 个展示模型）");
  }
  return parts.join("\n");
}

export function ModelPickerModal({
  appId,
  appName,
  appRunning,
  onClose,
  onSaved,
}: ModelPickerModalProps) {
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [data, setData] = useState<ClientModelsResponse | null>(null);
  const [mode, setMode] = useState<ModelFilterMode>("recommended");
  const [customIds, setCustomIds] = useState<string[]>([]);
  const [activeCategory, setActiveCategory] = useState<CategoryFilter>("__all__");
  const [sortBy, setSortBy] = useState<ModelSortBy>("default");
  const [search, setSearch] = useState("");

  const fetchModels = useCallback(async (targetAppId: AppId) => {
    setLoading(true);
    setErrorMsg(null);
    try {
      const res = await api.listClientModels(targetAppId);
      setData(res);
      setMode(res.filter.mode);
      if (res.filter.mode === "custom") {
        const lowerSet = new Set(res.filter.enabledIds.map((id) => id.toLowerCase()));
        const matched = res.models
          .filter((m) => lowerSet.has(m.id.toLowerCase()))
          .map((m) => m.id);
        setCustomIds(
          matched.length > 0
            ? matched
            : res.models.filter((m) => m.visible).map((m) => m.id),
        );
      } else {
        setCustomIds(res.models.filter((m) => m.visible).map((m) => m.id));
      }
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!appId) {
      setData(null);
      setSearch("");
      setActiveCategory("__all__");
      setSortBy("default");
      setErrorMsg(null);
      return;
    }
    setSearch("");
    setActiveCategory("__all__");
    setSortBy("default");
    void fetchModels(appId);
  }, [appId, fetchModels]);

  useEffect(() => {
    if (!appId) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !saving) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [appId, saving, onClose]);

  const recommendedLowerSet = useMemo(() => {
    return new Set((data?.recommendedIds ?? []).map((id) => id.toLowerCase()));
  }, [data]);

  const customLowerSet = useMemo(() => {
    return new Set(customIds.map((id) => id.toLowerCase()));
  }, [customIds]);

  const isModelChecked = useCallback(
    (m: ClientModelOption) => {
      if (mode === "all") return true;
      if (mode === "recommended") {
        return m.recommended || recommendedLowerSet.has(m.id.toLowerCase());
      }
      return customLowerSet.has(m.id.toLowerCase());
    },
    [mode, recommendedLowerSet, customLowerSet],
  );

  const checkedIds = useMemo(() => {
    if (!data) return [];
    return data.models.filter(isModelChecked).map((m) => m.id);
  }, [data, isModelChecked]);

  const familyStats = useMemo(() => {
    if (!data) return [];
    const list: { family: string; total: number; checked: number }[] = [];
    const idxMap = new Map<string, number>();
    for (const m of data.models) {
      let idx = idxMap.get(m.family);
      if (idx === undefined) {
        idx = list.length;
        idxMap.set(m.family, idx);
        list.push({ family: m.family, total: 0, checked: 0 });
      }
      list[idx].total += 1;
      if (isModelChecked(m)) {
        list[idx].checked += 1;
      }
    }
    return list;
  }, [data, isModelChecked]);

  const visibleModels = useMemo(() => {
    if (!data) return [];
    const q = search.trim().toLowerCase();
    const filtered = data.models.filter((m) => {
      if (activeCategory === "__selected__" && !isModelChecked(m)) {
        return false;
      }
      if (
        activeCategory !== "__all__" &&
        activeCategory !== "__selected__" &&
        m.family !== activeCategory
      ) {
        return false;
      }
      if (!q) return true;
      return (
        m.id.toLowerCase().includes(q) ||
        m.displayName.toLowerCase().includes(q) ||
        m.family.toLowerCase().includes(q) ||
        m.description.toLowerCase().includes(q)
      );
    });

    if (sortBy === "default") {
      return filtered;
    }

    return [...filtered].sort((a, b) => {
      if (sortBy === "price") {
        const pa = getEffectiveSortPrice(a) ?? Number.POSITIVE_INFINITY;
        const pb = getEffectiveSortPrice(b) ?? Number.POSITIVE_INFINITY;
        return pa - pb;
      }
      const sa = a.avgTps != null && a.avgTps > 0 ? a.avgTps : -1;
      const sb = b.avgTps != null && b.avgTps > 0 ? b.avgTps : -1;
      return sb - sa;
    });
  }, [data, activeCategory, search, sortBy, isModelChecked]);

  const handleModeSwitch = (nextMode: ModelFilterMode) => {
    if (!data) {
      setMode(nextMode);
      return;
    }
    if (nextMode === "recommended") {
      const rec = data.models
        .filter((m) => m.recommended || recommendedLowerSet.has(m.id.toLowerCase()))
        .map((m) => m.id);
      if (rec.length > 0) setCustomIds(rec);
    } else if (nextMode === "all") {
      setCustomIds(data.models.map((m) => m.id));
    } else if (nextMode === "custom") {
      const current =
        checkedIds.length > 0 ? checkedIds : data.models.slice(0, 1).map((m) => m.id);
      setCustomIds(current);
    }
    setMode(nextMode);
  };

  const handleToggleModel = (model: ClientModelOption) => {
    if (!data || saving) return;
    const currentlyChecked = isModelChecked(model);
    if (currentlyChecked && checkedIds.length <= 1) {
      return;
    }
    const baseSet = new Set(checkedIds.map((id) => id.toLowerCase()));
    if (currentlyChecked) {
      baseSet.delete(model.id.toLowerCase());
    } else {
      baseSet.add(model.id.toLowerCase());
    }
    const ordered = data.models
      .filter((m) => baseSet.has(m.id.toLowerCase()))
      .map((m) => m.id);
    if (ordered.length === 0) return;
    setCustomIds(ordered);
    setMode("custom");
  };

  const visibleCheckedCount = useMemo(
    () => visibleModels.filter(isModelChecked).length,
    [visibleModels, isModelChecked],
  );
  const allVisibleChecked =
    visibleModels.length > 0 && visibleCheckedCount === visibleModels.length;

  const remainingAfterUncheckVisible = useMemo(() => {
    if (!data || !allVisibleChecked) return checkedIds.length;
    const visibleLower = new Set(visibleModels.map((m) => m.id.toLowerCase()));
    return checkedIds.filter((id) => !visibleLower.has(id.toLowerCase())).length;
  }, [data, allVisibleChecked, checkedIds, visibleModels]);

  const canUncheckVisible = allVisibleChecked && remainingAfterUncheckVisible >= 1;

  const handleBatchToggleVisible = () => {
    if (!data || saving || visibleModels.length === 0) return;
    const baseSet = new Set(checkedIds.map((id) => id.toLowerCase()));
    if (allVisibleChecked) {
      if (!canUncheckVisible) return;
      for (const m of visibleModels) {
        baseSet.delete(m.id.toLowerCase());
      }
    } else {
      for (const m of visibleModels) {
        baseSet.add(m.id.toLowerCase());
      }
    }
    const ordered = data.models
      .filter((m) => baseSet.has(m.id.toLowerCase()))
      .map((m) => m.id);
    if (ordered.length === 0) return;
    setCustomIds(ordered);
    setMode("custom");
  };

  const handleResetDefault = () => {
    if (!appId || !data) return;
    const defaultMode: ModelFilterMode = appId === "claude" ? "recommended" : "all";
    handleModeSwitch(defaultMode);
  };

  const handleSave = async (andRestart: boolean) => {
    if (!appId || !data || saving) return;
    if (mode === "custom" && checkedIds.length === 0) {
      setErrorMsg("至少保留 1 个展示模型");
      return;
    }

    setSaving(true);
    setErrorMsg(null);
    const payloadFilter: ClientModelFilter = {
      mode,
      enabledIds: mode === "custom" ? checkedIds : data.recommendedIds,
    };

    try {
      const nextState = await api.saveClientModelFilter(appId, payloadFilter);
      if (andRestart) {
        await api.restartApp(appId);
      }
      onSaved(appId, nextState, andRestart);
    } catch (err) {
      setErrorMsg(`保存失败：${String(err)}`);
    } finally {
      setSaving(false);
    }
  };

  if (!appId) return null;

  return (
    <div
      className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-3"
      onClick={() => !saving && onClose()}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-[min(680px,calc(100vw-36px))] h-[min(486px,calc(100vh-30px))] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] flex flex-col overflow-hidden"
      >
        {/* Single-Row Native Header */}
        <div className="px-4 py-2.5 border-b border-[var(--border)] flex items-center justify-between gap-3 shrink-0">
          <div className="min-w-0 flex items-center gap-2.5">
            <AppBrandIcon appId={appId} className="h-7 w-7" />
            <h3 className="text-[14px] font-semibold tracking-[-0.01em] text-[var(--text-primary)] truncate">
              {appName} 模型管理
            </h3>
            {data && (
              <span className="tabular-nums text-[11px] font-medium px-2 py-0.5 rounded-full bg-[var(--accent-soft)] text-[var(--accent)] border border-[var(--accent-border)] shrink-0">
                已选 {checkedIds.length} / {data.totalCount}
              </span>
            )}
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <div className="grid grid-cols-3 p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)]">
              {(
                [
                  { id: "recommended", label: "推荐", Icon: Sparkles },
                  { id: "custom", label: "自选", Icon: SlidersHorizontal },
                  { id: "all", label: "全部", Icon: Layers },
                ] as const
              ).map(({ id, label, Icon }) => {
                const active = mode === id;
                return (
                  <button
                    key={id}
                    type="button"
                    disabled={loading || saving || !data}
                    onClick={() => handleModeSwitch(id)}
                    className={`h-[26px] px-2.5 rounded-[7px] text-[12px] inline-flex items-center justify-center gap-1 transition cursor-pointer disabled:opacity-60 ${
                      active
                        ? "bg-[var(--bg-card)] text-[var(--accent)] font-medium shadow-[var(--shadow-segment)]"
                        : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    <Icon className="h-3 w-3 shrink-0" />
                    <span>{label}</span>
                  </button>
                );
              })}
            </div>

            <button
              type="button"
              disabled={loading || saving}
              onClick={() => void fetchModels(appId)}
              title="重新拉取上游模型列表"
              className="h-7 w-7 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] flex items-center justify-center cursor-pointer disabled:opacity-50"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
            </button>
            <button
              type="button"
              disabled={saving}
              onClick={onClose}
              title="关闭"
              className="h-7 w-7 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] flex items-center justify-center cursor-pointer disabled:opacity-50"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* Split-Pane Body */}
        <div className="flex-1 min-h-0 flex overflow-hidden">
          {/* Left Category Rail */}
          <aside className="w-[156px] shrink-0 border-r border-[var(--border)] bg-[var(--bg-canvas)]/65 p-2 flex flex-col gap-0.5 overflow-y-auto subtle-scroll">
            <button
              type="button"
              onClick={() => setActiveCategory("__all__")}
              className={`w-full px-2 py-1.5 rounded-[8px] flex items-center justify-between gap-2 text-[12px] transition cursor-pointer ${
                activeCategory === "__all__"
                  ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)] border border-[var(--border)]"
                  : "border border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]/70"
              }`}
            >
              <span className="inline-flex items-center gap-2 min-w-0">
                <span className="h-5 w-5 rounded-[6px] bg-[var(--bg-subtle)] text-[var(--text-secondary)] flex items-center justify-center shrink-0">
                  <LayoutGrid className="h-3 w-3" />
                </span>
                <span className="truncate">全部模型</span>
              </span>
              {data && (
                <span className="tabular-nums text-[11px] text-[var(--text-muted)] shrink-0">
                  {data.totalCount}
                </span>
              )}
            </button>

            <button
              type="button"
              onClick={() => setActiveCategory("__selected__")}
              className={`w-full px-2 py-1.5 rounded-[8px] flex items-center justify-between gap-2 text-[12px] transition cursor-pointer ${
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
                  {checkedIds.length}
                </span>
              )}
            </button>

            {familyStats.length > 0 && (
              <div className="my-1 border-t border-[var(--border)]" />
            )}

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
                      stat.checked > 0 ? "text-[var(--accent)] font-medium" : "text-[var(--text-muted)]"
                    }`}
                  >
                    {stat.checked}/{stat.total}
                  </span>
                </button>
              );
            })}
          </aside>

          {/* Right Content Pane */}
          <div className="flex-1 min-w-0 flex flex-col bg-[var(--bg-card)]">
            {/* Search + Sort Pill + Dynamic Group Batch Action */}
            <div className="px-3 py-2 border-b border-[var(--border)] flex items-center gap-2 shrink-0">
              <div className="relative flex-1 min-w-0">
                <Search className="h-3.5 w-3.5 text-[var(--text-muted)] absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="搜索模型名、ID 或用途..."
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

              <div className="h-8 grid grid-cols-3 p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)] shrink-0">
                {(
                  [
                    { id: "default", label: "默认", Icon: null },
                    { id: "price", label: "便宜", Icon: Coins },
                    { id: "speed", label: "速度", Icon: Gauge },
                  ] as const
                ).map(({ id, label, Icon }) => {
                  const active = sortBy === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      disabled={!data}
                      onClick={() => setSortBy(id)}
                      title={
                        id === "default"
                          ? "按官方推荐顺序列出"
                          : id === "price"
                            ? "按近7日均价（无用量回退标准负载价）从便宜到贵排序"
                            : "按实测生成速度从快到慢排序"
                      }
                      className={`px-2 rounded-[7px] text-[11px] inline-flex items-center justify-center gap-1 transition cursor-pointer disabled:opacity-50 ${
                        active
                          ? "bg-[var(--bg-card)] text-[var(--accent)] font-medium shadow-[var(--shadow-segment)]"
                          : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      {Icon && <Icon className="h-3 w-3 shrink-0" />}
                      <span>{label}</span>
                    </button>
                  );
                })}
              </div>

              <button
                type="button"
                disabled={
                  !data ||
                  saving ||
                  visibleModels.length === 0 ||
                  (allVisibleChecked && !canUncheckVisible)
                }
                title={
                  allVisibleChecked && !canUncheckVisible
                    ? "至少保留 1 个展示模型"
                    : allVisibleChecked
                      ? "取消勾选当前列表中的所有模型"
                      : "全选当前列表中的所有模型"
                }
                onClick={handleBatchToggleVisible}
                className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1 cursor-pointer disabled:opacity-45 disabled:cursor-not-allowed shrink-0"
              >
                <CheckCheck className="h-3.5 w-3.5" />
                <span>{allVisibleChecked ? "取消当前" : "全选当前"}</span>
              </button>
            </div>

            {/* Scrollable 2-Column Rich Model Card Grid */}
            <div className="flex-1 min-h-0 overflow-y-auto subtle-scroll p-3">
              {errorMsg && (
                <div
                  className="mb-3 rounded-[10px] px-3 py-2 text-[12px] flex items-center justify-between gap-2 border"
                  style={{
                    backgroundColor: "var(--danger-soft)",
                    borderColor: "rgba(220, 38, 38, 0.24)",
                    color: "var(--danger)",
                  }}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">{errorMsg}</span>
                  </div>
                  {!data && (
                    <button
                      type="button"
                      onClick={() => void fetchModels(appId)}
                      className="px-2 py-0.5 rounded bg-[var(--bg-card)] text-[11px] font-medium cursor-pointer shrink-0"
                    >
                      重试
                    </button>
                  )}
                </div>
              )}

              {loading ? (
                <div className="h-full min-h-[220px] flex flex-col items-center justify-center text-[12px] text-[var(--text-secondary)] gap-2">
                  <RefreshCw className="h-4 w-4 animate-spin text-[var(--accent)]" />
                  <span>正在拉取 {appName} 可用模型列表...</span>
                </div>
              ) : data && visibleModels.length === 0 ? (
                <div className="h-full min-h-[220px] flex flex-col items-center justify-center text-[12px] text-[var(--text-muted)] gap-1.5">
                  <span>
                    {search.trim()
                      ? `没有匹配 “${search.trim()}” 的模型`
                      : "当前分类下暂无模型"}
                  </span>
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-2 content-start">
                  {visibleModels.map((model) => {
                    const checked = isModelChecked(model);
                    const lockedLast = checked && checkedIds.length <= 1;
                    const ctxLabel = formatContextWindow(model.contextWindow);
                    const priceBadge = getPriceBadge(model);
                    const speedBadge = getSpeedBadge(model);
                    return (
                      <button
                        key={model.id}
                        type="button"
                        disabled={saving || lockedLast}
                        title={buildCardTitle(model, lockedLast)}
                        onClick={() => handleToggleModel(model)}
                        className={`rounded-[11px] border p-2.5 text-left transition-all flex flex-col gap-1.5 ${
                          lockedLast ? "cursor-not-allowed" : "cursor-pointer"
                        } ${
                          checked
                            ? "bg-[var(--accent-soft)]/55 border-[var(--accent-border)] shadow-[0_1px_2px_rgba(79,70,229,0.08)]"
                            : "bg-[var(--bg-canvas)]/45 border-[var(--border)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]/55"
                        }`}
                      >
                        {/* Row 1: Vendor/Tier Icon + Name + Star + Check Circle */}
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0 flex items-center gap-2">
                            <ModelFamilyIcon family={model.family} size="md" />
                            <div className="min-w-0 flex items-center gap-1">
                              <span className="text-[12.5px] font-semibold text-[var(--text-primary)] truncate">
                                {model.displayName}
                              </span>
                              {model.recommended && (
                                <span
                                  title="推荐模型"
                                  className="inline-flex shrink-0 text-amber-500"
                                >
                                  <Star className="h-3 w-3 fill-current" />
                                </span>
                              )}
                            </div>
                          </div>

                          <span
                            className={`h-[18px] w-[18px] rounded-full border flex items-center justify-center shrink-0 transition ${
                              checked
                                ? "bg-[var(--accent)] border-[var(--accent)] text-white"
                                : "border-[var(--border-strong)] bg-[var(--bg-card)]"
                            }`}
                          >
                            {checked &&
                              (lockedLast ? (
                                <Lock className="h-2.5 w-2.5" />
                              ) : (
                                <Check className="h-3 w-3 stroke-[2.5]" />
                              ))}
                          </span>
                        </div>

                        {/* Row 2: Description */}
                        <div className="text-[11px] text-[var(--text-secondary)] truncate leading-snug">
                          {model.description}
                        </div>

                        {/* Row 3: Price + Speed (or ID fallback) & Capability Badges */}
                        <div className="flex items-center justify-between gap-1.5 pt-0.5">
                          {priceBadge || speedBadge ? (
                            <div className="flex items-center gap-1 min-w-0 overflow-hidden">
                              {priceBadge && (
                                <span
                                  style={priceBadge.style}
                                  className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-medium tabular-nums shrink-0"
                                >
                                  <Coins className="h-2.5 w-2.5 shrink-0" />
                                  <span>{priceBadge.label}</span>
                                </span>
                              )}
                              {speedBadge && (
                                <span
                                  style={speedBadge.style}
                                  className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-medium tabular-nums shrink-0"
                                >
                                  {speedBadge.tier === "fast" ? (
                                    <Zap className="h-2.5 w-2.5 shrink-0" />
                                  ) : speedBadge.tier === "medium" ? (
                                    <Gauge className="h-2.5 w-2.5 shrink-0" />
                                  ) : (
                                    <Clock className="h-2.5 w-2.5 shrink-0" />
                                  )}
                                  <span>{speedBadge.label}</span>
                                </span>
                              )}
                            </div>
                          ) : (
                            <code className="font-mono text-[10px] text-[var(--text-muted)] truncate min-w-0">
                              {model.id}
                            </code>
                          )}
                          <div className="flex items-center gap-1 shrink-0">
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
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Compact Footer */}
        <div className="px-4 py-2.5 border-t border-[var(--border)] flex items-center justify-between gap-3 shrink-0 bg-[var(--bg-card)]">
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <span
              className="inline-flex items-center gap-1.5 text-[11px] text-[var(--text-muted)] min-w-0"
              title={CLIENT_HINTS[appId]}
            >
              <Info className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{CLIENT_HINTS[appId]}</span>
            </span>
            <button
              type="button"
              disabled={!data || saving}
              onClick={handleResetDefault}
              className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11.5px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer disabled:opacity-50 shrink-0"
            >
              <RotateCcw className="h-3 w-3" />
              <span>恢复默认</span>
            </button>
          </div>

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
              disabled={!data || saving || checkedIds.length === 0}
              onClick={() => void handleSave(false)}
              className={`h-8 px-3 rounded-[9px] text-[12px] font-medium cursor-pointer disabled:opacity-50 ${
                appRunning
                  ? "border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)]"
                  : "bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white"
              }`}
            >
              {saving ? "保存中..." : "保存"}
            </button>
            {appRunning && (
              <button
                type="button"
                disabled={!data || saving || checkedIds.length === 0}
                onClick={() => void handleSave(true)}
                className="h-8 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium inline-flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
              >
                <RotateCw className="h-3.5 w-3.5" />
                <span>{saving ? "处理中..." : "保存并重启"}</span>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
