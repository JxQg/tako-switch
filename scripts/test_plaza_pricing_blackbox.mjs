import assert from "node:assert/strict";

const plaza = await import("../src/lib/plaza-pricing.ts");

const {
  PLAZA_INTERNAL_USD_PER_CNY,
  EXTERNAL_USD_TO_CNY,
  OFFICIAL_PRICE_RATIO_BASE,
  toPlazaCNY,
  formatPlazaMoney,
  channelRatioToOfficialMultiplier,
  formatMultiplier,
  formatZhe,
  matchesPlazaQuickFilter,
} = plaza;

function assertApprox(actual, expected, eps = 1e-12, message = "") {
  assert.equal(typeof actual, "number", `${message} expected number, got ${typeof actual}`);
  assert.ok(
    Math.abs(actual - expected) <= eps,
    `${message ? `${message}: ` : ""}expected ~${expected} (±${eps}), got ${actual}`,
  );
}

function makeModel(overrides = {}) {
  return {
    id: "gpt-6.2-sol",
    displayName: "GPT 6.2 Sol",
    family: "GPT",
    description: "General purpose flagship model",
    contextWindow: 200_000,
    recommended: false,
    visible: true,
    effortLevels: [],
    isFree: false,
    isDynamicPricing: false,
    standardLoadUsdPerM: 5.0,
    standardLoadMaxUsdPerM: null,
    realizedUsdPerM: 3.6,
    cacheHitRate: 0.5,
    inputUsdPerM: 2.5,
    outputUsdPerM: 10.0,
    cacheUsdPerM: 0.25,
    requestUsd: null,
    quotaType: 0,
    modelRatio: 1.25,
    officialMultiplier: 1.0,
    promoEndsOn: null,
    capabilities: ["chat"],
    tags: [],
    vendorName: "OpenAI",
    channels: [],
    avgTps: null,
    avgLatencyMs: null,
    successRate: null,
    ...overrides,
  };
}

let passed = 0;
function check(name, fn) {
  fn();
  passed += 1;
  console.log(`  [PASS] ${name}`);
}

console.log("Running Tako Switch v2.0.8 Plaza Pricing Blackbox Suite...");

// TC-V208-15: Constants & CNY conversion
check("TC-V208-15 constants & toPlazaCNY", () => {
  assert.equal(PLAZA_INTERNAL_USD_PER_CNY, 3);
  if (EXTERNAL_USD_TO_CNY !== undefined) {
    assert.equal(EXTERNAL_USD_TO_CNY, 6.8);
  }
  assert.equal(OFFICIAL_PRICE_RATIO_BASE, 20.4);

  assert.equal(toPlazaCNY(3), 1);
  assert.equal(toPlazaCNY(0), 0);
  assertApprox(toPlazaCNY(1.2), 0.4, 1e-12, "toPlazaCNY(1.2)");
});

// TC-V208-16: formatPlazaMoney across currencies, M/K units, perToken:false, zero & tiny values
check("TC-V208-16 formatPlazaMoney currency & M/K scaling", () => {
  assert.equal(formatPlazaMoney(null), null);
  assert.equal(formatPlazaMoney(undefined), null);

  // Default: currency = "cny", tokenUnit = "M", perToken = true
  assert.equal(formatPlazaMoney(3), "¥1");
  assert.equal(formatPlazaMoney(1.2), "¥0.4");

  // CNY with M and K
  assert.equal(formatPlazaMoney(6, { currency: "cny", tokenUnit: "M" }), "¥2");
  assert.equal(formatPlazaMoney(6, { currency: "cny", tokenUnit: "K" }), "¥0.002");

  // Internal USD with M and K
  assert.equal(formatPlazaMoney(6, { currency: "internal-usd", tokenUnit: "M" }), "$6");
  assert.equal(formatPlazaMoney(6, { currency: "internal-usd", tokenUnit: "K" }), "$0.006");

  // Per-request billing (`perToken: false`) MUST NEVER divide by 1000 under `tokenUnit: "K"`
  assert.equal(
    formatPlazaMoney(0.12, { currency: "cny", tokenUnit: "K", perToken: false }),
    "¥0.04",
  );
  assert.equal(
    formatPlazaMoney(0.12, { currency: "internal-usd", tokenUnit: "K", perToken: false }),
    "$0.12",
  );
  assert.equal(
    formatPlazaMoney(0.12, { currency: "cny", tokenUnit: "M", perToken: false }),
    "¥0.04",
  );

  // Zero, >= 100 integer rounding, and tiny values without scientific notation
  assert.equal(formatPlazaMoney(0, { currency: "cny", tokenUnit: "M" }), "¥0");
  assert.equal(formatPlazaMoney(0, { currency: "internal-usd", tokenUnit: "K" }), "$0");
  assert.equal(formatPlazaMoney(300, { currency: "cny", tokenUnit: "M" }), "¥100");

  const tinyCny = formatPlazaMoney(0.00012, { currency: "cny", tokenUnit: "M" });
  assert.equal(tinyCny, "¥0.00004");
  assert.ok(!tinyCny.toLowerCase().includes("e"), `scientific notation forbidden: ${tinyCny}`);

  const tinyUsd = formatPlazaMoney(0.00004, { currency: "internal-usd", tokenUnit: "M" });
  assert.equal(tinyUsd, "$0.00004");
  assert.ok(!tinyUsd.toLowerCase().includes("e"), `scientific notation forbidden: ${tinyUsd}`);
});

// TC-V208-17: Official multiplier & zhe formatting
check("TC-V208-17 channelRatioToOfficialMultiplier, formatMultiplier & formatZhe", () => {
  assertApprox(channelRatioToOfficialMultiplier(2.04), 0.1, 1e-12, "ratio 2.04 -> 0.1x");
  assertApprox(channelRatioToOfficialMultiplier(1.224), 0.06, 1e-12, "ratio 1.224 -> 0.06x");

  assert.equal(formatMultiplier(0.06), "0.06");
  assert.equal(formatZhe(0.06), "0.6");
});

// TC-V208-18: matchesPlazaQuickFilter all 6 scenarios + exact boundary cases
check("TC-V208-18 matchesPlazaQuickFilter 6 scenarios & boundaries", () => {
  const plainModel = makeModel();
  const promoModel = makeModel({ id: "gemini-4-carbon", promoEndsOn: "2026-11-30" });
  const cheapBoundaryHit = makeModel({ officialMultiplier: 0.9 });
  const cheapLowHit = makeModel({ officialMultiplier: 0.06 });
  const cheapBoundaryMiss = makeModel({ officialMultiplier: 0.91 });
  const cheapNullMiss = makeModel({ officialMultiplier: null, quotaType: 1 });
  const codeModel = makeModel({ id: "swe-2", capabilities: ["chat", "code"] });
  const longBoundaryHit = makeModel({ contextWindow: 1_000_000 });
  const longBoundaryMiss = makeModel({ contextWindow: 999_999 });
  const longNullMiss = makeModel({ contextWindow: null });
  const imageModel = makeModel({ id: "gemini-3.1-flash-image", capabilities: ["image"] });

  // 1) "all"
  assert.equal(matchesPlazaQuickFilter(plainModel, "all"), true);
  assert.equal(matchesPlazaQuickFilter(imageModel, "all"), true);

  // 2) "promo"
  assert.equal(matchesPlazaQuickFilter(promoModel, "promo"), true);
  assert.equal(matchesPlazaQuickFilter(plainModel, "promo"), false);

  // 3) "cheap" (officialMultiplier <= 0.9)
  assert.equal(matchesPlazaQuickFilter(cheapLowHit, "cheap"), true);
  assert.equal(matchesPlazaQuickFilter(cheapBoundaryHit, "cheap"), true);
  assert.equal(matchesPlazaQuickFilter(cheapBoundaryMiss, "cheap"), false);
  assert.equal(matchesPlazaQuickFilter(cheapNullMiss, "cheap"), false);

  // 4) "code"
  assert.equal(matchesPlazaQuickFilter(codeModel, "code"), true);
  assert.equal(matchesPlazaQuickFilter(plainModel, "code"), false);

  // 5) "long" (contextWindow >= 1_000_000)
  assert.equal(matchesPlazaQuickFilter(longBoundaryHit, "long"), true);
  assert.equal(matchesPlazaQuickFilter(longBoundaryMiss, "long"), false);
  assert.equal(matchesPlazaQuickFilter(longNullMiss, "long"), false);

  // 6) "image"
  assert.equal(matchesPlazaQuickFilter(imageModel, "image"), true);
  assert.equal(matchesPlazaQuickFilter(plainModel, "image"), false);
});

console.log(`All ${passed} Plaza pricing blackbox test groups passed.`);
