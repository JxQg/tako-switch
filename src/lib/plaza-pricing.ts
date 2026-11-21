import type { ClientModelOption } from "./api";

export const PLAZA_INTERNAL_USD_PER_CNY = 3;
export const EXTERNAL_USD_TO_CNY = 6.8;
export const OFFICIAL_PRICE_RATIO_BASE = 20.4; // EXTERNAL_USD_TO_CNY * PLAZA_INTERNAL_USD_PER_CNY

export const PLAZA_CURRENCY_STORAGE_KEY = "tako-switch-plaza-currency";
export const PLAZA_UNIT_STORAGE_KEY = "tako-switch-plaza-unit";
export const PLAZA_PRICE_MODE_STORAGE_KEY = "tako-switch-plaza-price-mode";

export type PlazaCurrency = "cny" | "internal-usd";
export type TokenUnit = "M" | "K";
export type PlazaPriceMode = "detail" | "avg";
export type PlazaQuickFilterId = "all" | "promo" | "cheap" | "code" | "long" | "image";
export type ModelCapability =
  | "chat"
  | "reason"
  | "vision"
  | "code"
  | "image"
  | "video"
  | "audio"
  | "embed";

export function toPlazaCNY(internalUSD: number): number {
  return internalUSD / PLAZA_INTERNAL_USD_PER_CNY;
}

export function formatPlazaNumber(value: number): string {
  const abs = Math.abs(value);
  if (abs === 0) return "0";
  if (abs >= 100) return value.toFixed(0);
  if (abs >= 1) return Number(value.toFixed(2)).toString();

  const prec = value.toPrecision(3);
  if (!prec.includes("e") && !prec.includes("E")) {
    const parsed = Number(prec).toString();
    if (!parsed.includes("e") && !parsed.includes("E")) {
      return parsed;
    }
  }
  const fixed = value.toFixed(10).replace(/\.?0+$/, "");
  return fixed === "" || fixed === "-0" ? "0" : fixed;
}

export function formatPlazaMoney(
  internalUSD: number | null | undefined,
  options?: {
    currency?: PlazaCurrency;
    tokenUnit?: TokenUnit;
    perToken?: boolean;
  },
): string | null {
  if (internalUSD == null || !Number.isFinite(internalUSD) || internalUSD < 0) {
    return null;
  }
  const currency = options?.currency ?? "cny";
  const tokenUnit = options?.tokenUnit ?? "M";
  const perToken = options?.perToken !== false;

  let converted = currency === "cny" ? toPlazaCNY(internalUSD) : internalUSD;
  if (perToken && tokenUnit === "K") {
    converted = converted / 1000;
  }
  const symbol = currency === "cny" ? "¥" : "$";
  return `${symbol}${formatPlazaNumber(converted)}`;
}

export function channelRatioToOfficialMultiplier(ratio: number): number {
  return ratio / OFFICIAL_PRICE_RATIO_BASE;
}

export function formatMultiplier(mult: number | null | undefined): string | null {
  if (mult == null || !Number.isFinite(mult) || mult <= 0) return null;
  if (mult >= 10) return Number(mult.toFixed(1)).toString();
  if (mult >= 0.01) return Number(mult.toFixed(2)).toString();
  const prec = mult.toPrecision(2);
  if (!prec.includes("e") && !prec.includes("E")) {
    const parsed = Number(prec).toString();
    if (!parsed.includes("e") && !parsed.includes("E")) return parsed;
  }
  return mult.toFixed(6).replace(/\.?0+$/, "") || "0";
}

export function formatZhe(mult: number | null | undefined): string | null {
  if (mult == null || !Number.isFinite(mult) || mult <= 0) return null;
  const zhe = mult * 10;
  if (zhe >= 0.1) return Number(zhe.toFixed(1)).toString();
  if (zhe >= 0.01) return Number(zhe.toFixed(2)).toString();
  return zhe.toFixed(4).replace(/\.?0+$/, "") || "0";
}

export function formatOfficialDiscountBadge(
  mult: number | null | undefined,
): string | null {
  if (mult == null || !Number.isFinite(mult) || mult <= 0) return null;
  const mText = formatMultiplier(mult);
  if (!mText) return null;
  if (mult < 0.995) {
    const zText = formatZhe(mult);
    return zText ? `≈ ${mText}x 官方 ${zText}折` : `≈ ${mText}x 官方`;
  }
  if (mult <= 1.005) {
    return "≈ 1x 官方平价";
  }
  return `≈ ${mText}x 官方`;
}

export function formatPromoBadge(promoEndsOn: string | null | undefined): string | null {
  if (!promoEndsOn) return null;
  const trimmed = promoEndsOn.trim();
  if (!trimmed) return null;
  const shortDate = /^\d{4}-\d{2}-\d{2}$/.test(trimmed) ? trimmed.slice(5) : trimmed;
  return `🔥 限时半价 · ${shortDate}`;
}

export function matchesPlazaQuickFilter(
  model: Pick<
    ClientModelOption,
    "promoEndsOn" | "officialMultiplier" | "capabilities" | "contextWindow"
  >,
  quickFilter: PlazaQuickFilterId,
): boolean {
  switch (quickFilter) {
    case "all":
      return true;
    case "promo":
      return Boolean(model.promoEndsOn);
    case "cheap":
      return model.officialMultiplier != null && model.officialMultiplier <= 0.9;
    case "code":
      return Array.isArray(model.capabilities) && model.capabilities.includes("code");
    case "long":
      return (model.contextWindow ?? 0) >= 1_000_000;
    case "image":
      return Array.isArray(model.capabilities) && model.capabilities.includes("image");
    default:
      return true;
  }
}

export function readStoredPlazaCurrency(): PlazaCurrency {
  try {
    const v = window.localStorage.getItem(PLAZA_CURRENCY_STORAGE_KEY);
    if (v === "internal-usd" || v === "cny") return v;
  } catch {
    // ignore storage errors
  }
  return "cny";
}

export function writeStoredPlazaCurrency(currency: PlazaCurrency): void {
  try {
    window.localStorage.setItem(PLAZA_CURRENCY_STORAGE_KEY, currency);
  } catch {
    // ignore storage errors
  }
}

export function readStoredTokenUnit(): TokenUnit {
  try {
    const v = window.localStorage.getItem(PLAZA_UNIT_STORAGE_KEY);
    if (v === "K" || v === "M") return v;
  } catch {
    // ignore storage errors
  }
  return "M";
}

export function writeStoredTokenUnit(unit: TokenUnit): void {
  try {
    window.localStorage.setItem(PLAZA_UNIT_STORAGE_KEY, unit);
  } catch {
    // ignore storage errors
  }
}

export function readStoredPlazaPriceMode(): PlazaPriceMode {
  try {
    const v = window.localStorage.getItem(PLAZA_PRICE_MODE_STORAGE_KEY);
    if (v === "avg" || v === "detail") return v;
  } catch {
    // ignore storage errors
  }
  return "detail";
}

export function writeStoredPlazaPriceMode(mode: PlazaPriceMode): void {
  try {
    window.localStorage.setItem(PLAZA_PRICE_MODE_STORAGE_KEY, mode);
  } catch {
    // ignore storage errors
  }
}
