(function () {
  "use strict";

  const TABS = [
    { key: "overview", path: "/tako/win-lab", label: "门禁矩阵" },
    { key: "runs", path: "/tako/win-lab/runs", label: "测试记录" },
    { key: "candidates", path: "/tako/win-lab/candidates", label: "上游版本与日志" },
    { key: "hq", path: "/tako/win-lab/hq", label: "HQ 事件上报" },
  ];

  let state = {
    tab: "overview",
    overview: null,
    runs: [],
    selectedRun: null,
    candidates: null,
    hqEvents: [],
    hqStats: null,
    busy: false,
    toast: null,
    customApp: "codex",
    customUrl: "",
    customVer: "",
  };

  function inferTabFromPath(pathname) {
    const p = (pathname || "").replace(/^\/api\/plugins\/[^/]+/, "");
    if (p.includes("/runs")) return "runs";
    if (p.includes("/candidates")) return "candidates";
    if (p.includes("/hq")) return "hq";
    return "overview";
  }

  const embed = window.OpsConsoleEmbed.init({
    pluginId: "win-lab",
    onNavigate: function (path) {
      const next = inferTabFromPath(path);
      if (next !== state.tab) {
        state.tab = next;
        loadActiveTab();
      }
    },
  });

  state.tab = inferTabFromPath(window.location.pathname);

  function esc(v) {
    return String(v == null ? "" : v)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function badgeClass(status) {
    const s = String(status || "").toLowerCase();
    if (s === "pass" || s === "sent" || s === "approved" || s === "ok") return "b-pass";
    if (s === "warn" || s === "deduped" || s === "pinned_review_windows") return "b-warn";
    if (s === "fail" || s === "error" || s === "failed") return "b-fail";
    if (s === "running" || s === "queued") return "b-running";
    return "b-neutral";
  }

  function fmtTime(iso) {
    if (!iso) return "-";
    try {
      return new Date(iso).toLocaleString("zh-CN", { hour12: false });
    } catch {
      return iso;
    }
  }

  function setToast(msg, isErr) {
    state.toast = { msg, isErr: !!isErr };
    render();
    setTimeout(function () {
      if (state.toast && state.toast.msg === msg) {
        state.toast = null;
        render();
      }
    }, 5000);
  }

  async function apiGet(path) {
    const resp = await embed.apiFetch(path);
    const data = await resp.json();
    if (!resp.ok || data.ok === false) {
      throw new Error(data.error || "请求失败 (" + resp.status + ")");
    }
    return data.data;
  }

  async function apiPost(path, body) {
    const resp = await embed.apiFetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {}),
    });
    const data = await resp.json();
    if (!resp.ok || data.ok === false) {
      throw new Error(data.error || "操作失败 (" + resp.status + ")");
    }
    return data.data;
  }

  async function loadOverview() {
    state.overview = await apiGet("/api/overview");
  }

  async function loadRuns() {
    const d = await apiGet("/api/runs?limit=60");
    state.runs = (d && d.items) || [];
    const qsRun = new URLSearchParams(window.location.search).get("id");
    if (qsRun && !state.selectedRun) {
      await openRunDetail(qsRun);
    }
  }

  async function openRunDetail(runId) {
    state.selectedRun = await apiGet("/api/runs/" + encodeURIComponent(runId));
    render();
  }

  async function loadCandidates() {
    state.candidates = await apiGet("/api/candidates");
  }

  async function loadHq() {
    const d = await apiGet("/api/hq/events?limit=80");
    state.hqEvents = (d && d.items) || [];
    state.hqStats = d || {};
  }

  async function loadActiveTab() {
    try {
      await loadOverview();
      if (state.tab === "runs") await loadRuns();
      else if (state.tab === "candidates") await loadCandidates();
      else if (state.tab === "hq") await loadHq();
    } catch (err) {
      setToast(err.message || String(err), true);
    }
    render();
  }

  async function triggerGate(appId, opts) {
    state.busy = true;
    render();
    try {
      const payload = Object.assign({ app_id: appId, report_hq: true }, opts || {});
      const res = await apiPost("/api/runs/gate", payload);
      const count = (res && res.runs && res.runs.length) || 1;
      setToast("已入队 Windows 三门禁任务（" + appId + "，共 " + count + " 项）");
      await loadActiveTab();
    } catch (err) {
      setToast(err.message, true);
    } finally {
      state.busy = false;
      render();
    }
  }

  async function triggerSmoke(appId) {
    state.busy = true;
    render();
    try {
      const res = await apiPost("/api/runs/smoke", { app_id: appId, hold_seconds: 15, report_hq: true });
      setToast("已入队 Windows 实机冷启动冒烟：" + appId + " (" + ((res && res.run && res.run.id) || "") + ")");
      await loadActiveTab();
    } catch (err) {
      setToast(err.message, true);
    } finally {
      state.busy = false;
      render();
    }
  }

  async function triggerUpstreamCheck() {
    state.busy = true;
    render();
    try {
      await apiPost("/api/candidates/check", { report_hq: true });
      setToast("已触发上游新版与官方 Changelog 巡检");
      await loadActiveTab();
    } catch (err) {
      setToast(err.message, true);
    } finally {
      state.busy = false;
      render();
    }
  }

  async function triggerHqPing(createTicket) {
    state.busy = true;
    render();
    try {
      const res = await apiPost("/api/hq/report", {
        event_type: "winlab.gate_passed",
        title: "Windows 测试台 (.6) HQ 连通性自检",
        body: "由云运维控制台手动触发的 HQ 链路探活事件。",
        priority: "P3",
        create_ticket: !!createTicket,
      });
      const ev = res && res.event;
      setToast("HQ 上报完成: status=" + (ev && ev.status) + " http=" + (ev && ev.http_status));
      await loadHq();
    } catch (err) {
      setToast(err.message, true);
    } finally {
      state.busy = false;
      render();
    }
  }

  async function reportRunToHq(runId) {
    state.busy = true;
    render();
    try {
      const res = await apiPost("/api/hq/report", { run_id: runId });
      const ev = res && res.event;
      setToast("已上报至 HQ: " + (ev && ev.status) + (ev && ev.ticket_id ? " (工单 #" + ev.ticket_id + ")" : ""));
      await loadActiveTab();
    } catch (err) {
      setToast(err.message, true);
    } finally {
      state.busy = false;
      render();
    }
  }

  function renderGateChips(gateRun) {
    if (!gateRun || !gateRun.gates) return '<span class="badge b-neutral">未测</span>';
    const order = [
      ["localization", "汉化"],
      ["injection", "注入"],
      ["anomalyAudit", "审计"],
    ];
    return (
      '<div class="gate-chips">' +
      order
        .map(function (pair) {
          const g = gateRun.gates[pair[0]];
          const st = g ? g.status : "pending";
          return '<span class="badge ' + badgeClass(st) + '">' + pair[1] + ":" + esc(st) + "</span>";
        })
        .join("") +
      "</div>"
    );
  }

  function renderOverviewTab() {
    const ov = state.overview;
    if (!ov) return '<div class="lab-loading">加载总览中…</div>';
    const host = ov.host || {};
    const ups = ov.upstreams || {};
    const hqOk = ups.hq && ups.hq.ok;
    const telOk = ups.telemetry && ups.telemetry.ok;
    const badge = ov.badge || {};

    const kpis =
      '<div class="kpi-grid">' +
      '<div class="kpi-card"><div class="kpi-label">Windows 测试节点</div><div class="kpi-value mono">' +
      esc(host.hostname || ".6") +
      '</div><div class="kpi-sub">磁盘可用 ' +
      esc(host.diskFreeGB) +
      " GB / " +
      esc(host.diskTotalGB) +
      " GB</div></div>" +
      '<div class="kpi-card"><div class="kpi-label">门禁阻断 / 异常</div><div class="kpi-value num">' +
      esc(badge.failing_gates || 0) +
      '</div><div class="kpi-sub">运行中任务 ' +
      esc(badge.running_jobs || 0) +
      " 个</div></div>" +
      '<div class="kpi-card"><div class="kpi-label">待审上游新版</div><div class="kpi-value num">' +
      esc(badge.pending_candidates || 0) +
      '</div><div class="kpi-sub">' +
      esc((ov.latestUpstream && ov.latestUpstream.summary) || "未巡检") +
      "</div></div>" +
      '<div class="kpi-card"><div class="kpi-label">HQ / 遥测链路</div><div class="kpi-value"><span class="badge ' +
      (hqOk ? "b-pass" : "b-fail") +
      '">HQ ' +
      (hqOk ? "在线" : "离线") +
      '</span> <span class="badge ' +
      (telOk ? "b-pass" : "b-warn") +
      '">Telemetry ' +
      (telOk ? "在线" : "离线") +
      '</span></div><div class="kpi-sub">24h HQ 上报 ' +
      esc((ov.hq24h && ov.hq24h.sent) || 0) +
      " 条（去重 " +
      esc((ov.hq24h && ov.hq24h.deduped) || 0) +
      "）</div></div>" +
      "</div>";

    const rows = (ov.matrix || [])
      .map(function (m) {
        const g = m.latestGate;
        const s = m.latestSmoke;
        const smokeHtml = s
          ? '<span class="badge ' +
            badgeClass(s.status) +
            '">' +
            esc(s.status) +
            '</span><div class="muted">' +
            esc(s.summary || "") +
            "</div>"
          : '<span class="badge b-neutral">未运行</span>';
        return (
          "<tr>" +
          '<td><div class="app-name">' +
          esc(m.displayName) +
          '</div><div class="muted mono">' +
          esc(m.appId) +
          " · " +
          esc(m.installType || "-") +
          "</div></td>" +
          '<td><div class="mono">' +
          esc(m.pinnedVersion || "-") +
          '</div><div><span class="badge ' +
          badgeClass(m.pinnedStatus) +
          '">' +
          esc(m.pinnedStatus) +
          "</span>" +
          (m.hasUpstreamUpdate
            ? ' <span class="badge b-warn">上游 ' + esc(m.upstreamVersion) + "</span>"
            : "") +
          "</div></td>" +
          "<td>" +
          renderGateChips(g) +
          (g ? '<div class="muted">' + esc(fmtTime(g.finished_at || g.started_at)) + "</div>" : "") +
          "</td>" +
          "<td>" +
          smokeHtml +
          "</td>" +
          '<td><div class="btn-row">' +
          '<button class="btn" data-act="gate" data-app="' +
          esc(m.appId) +
          '" ' +
          (state.busy ? "disabled" : "") +
          ">跑三门禁</button>" +
          '<button class="btn btn-warn" data-act="smoke" data-app="' +
          esc(m.appId) +
          '" ' +
          (state.busy ? "disabled" : "") +
          ">实机启动测试</button>" +
          (g
            ? '<button class="btn" data-act="detail" data-id="' + esc(g.id) + '">详情</button>'
            : "") +
          "</div></td>" +
          "</tr>"
        );
      })
      .join("");

    const matrixPanel =
      '<div class="panel">' +
      '<div class="panel-head"><div><div class="panel-title">Windows 客户端 4 维发版门禁矩阵</div>' +
      '<div class="panel-sub">数据源: manifest/pinned-versions.json · 三门禁（汉化 / 注入 / 异常审计）+ Gate 4 实机冷启动与 EventLog 崩溃检测</div></div>' +
      '<div class="btn-row">' +
      '<button class="btn btn-primary" data-act="gate-all" ' +
      (state.busy ? "disabled" : "") +
      ">全量跑 Windows 三门禁</button>" +
      '<button class="btn" data-act="check-upstream" ' +
      (state.busy ? "disabled" : "") +
      ">巡检上游 Changelog</button>" +
      '<button class="btn" data-act="refresh">刷新</button>' +
      "</div></div>" +
      '<div class="table-wrap"><table><thead><tr>' +
      "<th>客户端</th><th>锁定版本 (Win x64)</th><th>静态三门禁</th><th>Gate 4 实机启动</th><th>操作</th>" +
      "</tr></thead><tbody>" +
      rows +
      "</tbody></table></div></div>";

    const customPanel =
      '<div class="panel">' +
      '<div class="panel-head"><div><div class="panel-title">候选安装包预验收（不改线上版本）</div>' +
      '<div class="panel-sub">输入上游候选安装包 URL，直接由 .6 隔离解包执行 Windows 三门禁验收</div></div></div>' +
      '<div class="form-inline">' +
      '<select class="input" id="cand-app" style="min-width:130px">' +
      ["codex", "claude", "antigravity", "dsh-tako"]
        .map(function (id) {
          return '<option value="' + id + '"' + (state.customApp === id ? " selected" : "") + ">" + id + "</option>";
        })
        .join("") +
      "</select>" +
      '<input class="input" id="cand-ver" placeholder="候选版本号 (如 26.1125.1)" value="' +
      esc(state.customVer) +
      '" />' +
      '<input class="input" id="cand-url" style="flex:1;min-width:260px" placeholder="Windows 安装包 URL (.msix / .exe)" value="' +
      esc(state.customUrl) +
      '" />' +
      '<button class="btn btn-primary" data-act="run-custom" ' +
      (state.busy ? "disabled" : "") +
      ">提交预验收</button>" +
      "</div></div>";

    return kpis + matrixPanel + customPanel;
  }

  function renderRunsTab() {
    const items = state.runs || [];
    const rows = items
      .map(function (r) {
        return (
          "<tr>" +
          '<td class="mono">' +
          esc(r.id) +
          "</td>" +
          "<td><strong>" +
          esc(r.app_id) +
          '</strong> <span class="muted">' +
          esc(r.kind) +
          "</span></td>" +
          '<td class="mono">' +
          esc(r.target_version || "-") +
          "</td>" +
          '<td><span class="badge ' +
          badgeClass(r.status) +
          '">' +
          esc(r.status) +
          "</span></td>" +
          "<td>" +
          esc(r.summary || "-") +
          (r.hq_ticket_id ? ' <span class="badge b-warn">HQ #' + esc(r.hq_ticket_id) + "</span>" : "") +
          "</td>" +
          '<td class="muted mono">' +
          esc(fmtTime(r.started_at)) +
          "</td>" +
          '<td><div class="btn-row">' +
          '<button class="btn" data-act="detail" data-id="' +
          esc(r.id) +
          '">日志</button>' +
          '<button class="btn" data-act="report-run" data-id="' +
          esc(r.id) +
          '">报 HQ</button>' +
          "</div></td>" +
          "</tr>"
        );
      })
      .join("");

    let detailHtml = "";
    if (state.selectedRun) {
      const r = state.selectedRun;
      detailHtml =
        '<div class="panel">' +
        '<div class="panel-head"><div><div class="panel-title">运行详情 · ' +
        esc(r.id) +
        " (" +
        esc(r.app_id) +
        " " +
        esc(r.target_version) +
        ')</div><div class="panel-sub">' +
        esc(r.summary) +
        '</div></div><button class="btn" data-act="close-detail">关闭</button></div>' +
        '<div class="pre-box mono">' +
        esc(JSON.stringify({ gates: r.gates, smoke: r.smoke, artifacts: r.artifacts }, null, 2)) +
        "</div>" +
        (r.log_text
          ? '<div style="margin-top:10px"><div class="panel-sub" style="margin-bottom:6px">执行日志</div><div class="pre-box mono">' +
            esc(r.log_text) +
            "</div></div>"
          : "") +
        "</div>";
    }

    return (
      detailHtml +
      '<div class="panel">' +
      '<div class="panel-head"><div><div class="panel-title">测试执行记录</div><div class="panel-sub">按时间倒序展示最近 60 次门禁、实机冒烟与上游巡检</div></div>' +
      '<button class="btn" data-act="refresh">刷新</button></div>' +
      '<div class="table-wrap"><table><thead><tr>' +
      "<th>Run ID</th><th>目标</th><th>版本</th><th>状态</th><th>结果摘要</th><th>开始时间</th><th>操作</th>" +
      "</tr></thead><tbody>" +
      (rows || '<tr><td colspan="7" class="muted">暂无执行记录</td></tr>') +
      "</tbody></table></div></div>"
    );
  }

  function renderCandidatesTab() {
    const snap = state.candidates || {};
    const rep = snap.report || {};
    const apps = rep.apps || {};
    const rows = Object.keys(apps)
      .map(function (aid) {
        const c = apps[aid] || {};
        const cl = c.changelog || {};
        const hints = (cl.riskHints || []).map(function (h) {
          return h.category;
        });
        return (
          "<tr>" +
          "<td><strong>" +
          esc(c.displayName || aid) +
          '</strong><div class="muted mono">' +
          esc(aid) +
          "</div></td>" +
          '<td class="mono">' +
          esc(c.pinnedVersion || "-") +
          "</td>" +
          '<td class="mono">' +
          esc(c.upstreamVersion || "-") +
          "</td>" +
          '<td><span class="badge ' +
          (c.hasUpstreamUpdate ? "b-warn" : "b-pass") +
          '">' +
          (c.hasUpstreamUpdate ? "发现新版待验" : "一致") +
          "</span></td>" +
          "<td>" +
          esc(hints.length ? hints.join(", ") : "无高危词命中") +
          "</td>" +
          "</tr>"
        );
      })
      .join("");

    return (
      '<div class="panel">' +
      '<div class="panel-head"><div><div class="panel-title">上游版本巡检与官方更新日志分析</div>' +
      '<div class="panel-sub">最近检查: ' +
      esc(fmtTime(snap.checked_at)) +
      " · 策略: 禁止自动跟进，必须审核日志并通过 macOS + Windows 门禁后 promote</div></div>" +
      '<button class="btn btn-primary" data-act="check-upstream" ' +
      (state.busy ? "disabled" : "") +
      ">立即巡检上游</button></div>' +
      '<div class="table-wrap"><table><thead><tr><th>客户端</th><th>当前锁定</th><th>上游最新</th><th>状态</th><th>Changelog 风险词</th></tr></thead><tbody>' +
      (rows || '<tr><td colspan="5" class="muted">尚未巡检，点击右上角立即拉取官方更新源</td></tr>') +
      "</tbody></table></div></div>" +
      (snap.report_md
        ? '<div class="panel"><div class="panel-title" style="margin-bottom:8px">完整风险分析报告</div><div class="pre-box mono">' +
          esc(snap.report_md) +
          "</div></div>"
        : "")
    );
  }

  function renderHqTab() {
    const items = state.hqEvents || [];
    const rows = items
      .map(function (e) {
        return (
          "<tr>" +
          '<td class="muted mono">' +
          esc(fmtTime(e.created_at)) +
          "</td>" +
          '<td class="mono">' +
          esc(e.event_type) +
          '<div class="muted">' +
          esc(e.channel) +
          "</div></td>" +
          "<td>" +
          esc(e.title) +
          "</td>" +
          '<td><span class="badge ' +
          badgeClass(e.status) +
          '">' +
          esc(e.status) +
          "</span>" +
          (e.http_status ? ' <span class="muted mono">HTTP ' + esc(e.http_status) + "</span>" : "") +
          "</td>" +
          '<td class="mono">' +
          (e.ticket_id ? "#" + esc(e.ticket_id) : "-") +
          "</td>" +
          '<td class="muted">' +
          esc((e.response_text || "").slice(0, 120)) +
          "</td>" +
          "</tr>"
        );
      })
      .join("");

    return (
      '<div class="panel">' +
      '<div class="panel-head"><div><div class="panel-title">HQ 事件与 Tako 遥测上报记录</div>' +
      '<div class="panel-sub">失败门禁 / 实机闪退自动建 HQ 工单（含冷却去重），通过事件走轻量 SSE；所有上报均经过脱敏</div></div>' +
      '<div class="btn-row">' +
      '<button class="btn btn-primary" data-act="hq-ping" ' +
      (state.busy ? "disabled" : "") +
      ">发送 HQ 连通性自检</button>" +
      '<button class="btn" data-act="refresh">刷新</button>' +
      "</div></div>" +
      '<div class="table-wrap"><table><thead><tr><th>时间</th><th>事件类型</th><th>标题</th><th>状态</th><th>HQ 工单</th><th>回执</th></tr></thead><tbody>' +
      (rows || '<tr><td colspan="6" class="muted">暂无上报记录</td></tr>') +
      "</tbody></table></div></div>"
    );
  }

  function render() {
    const root = document.getElementById("app");
    if (!root) return;

    const showLocalNav = !embed.hostNav;
    const navHtml = showLocalNav
      ? '<div class="topbar"><div><div class="brand-title">Windows 测试台 · Tako Win Lab</div>' +
        '<div class="brand-sub">河南 .6 (172.17.66.6:18795) · 客户端版本管控 / Windows 4 维验收 / HQ 事件上报</div></div>' +
        '<div class="pill-group">' +
        TABS.map(function (t) {
          return (
            '<button class="pill-btn' +
            (state.tab === t.key ? " active" : "") +
            '" data-tab="' +
            t.key +
            '">' +
            t.label +
            "</button>"
          );
        }).join("") +
        "</div></div>"
      : "";

    const toastHtml = state.toast
      ? '<div class="toast' + (state.toast.isErr ? " err" : "") + '">' + esc(state.toast.msg) + "</div>"
      : "";

    let bodyHtml = "";
    if (state.tab === "overview") bodyHtml = renderOverviewTab();
    else if (state.tab === "runs") bodyHtml = renderRunsTab();
    else if (state.tab === "candidates") bodyHtml = renderCandidatesTab();
    else if (state.tab === "hq") bodyHtml = renderHqTab();

    root.innerHTML =
      '<div class="lab-wrap' + (embed.isEmbed ? " is-embed" : "") + '">' + navHtml + toastHtml + bodyHtml + "</div>";

    root.querySelectorAll("[data-tab]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        const key = btn.getAttribute("data-tab");
        const found = TABS.find(function (t) {
          return t.key === key;
        });
        state.tab = key;
        if (found) embed.notifyRoute(found.path);
        loadActiveTab();
      });
    });

    root.querySelectorAll("[data-act]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        const act = btn.getAttribute("data-act");
        if (act === "refresh") loadActiveTab();
        else if (act === "gate") triggerGate(btn.getAttribute("data-app"));
        else if (act === "gate-all") triggerGate("all");
        else if (act === "smoke") triggerSmoke(btn.getAttribute("data-app"));
        else if (act === "check-upstream") triggerUpstreamCheck();
        else if (act === "hq-ping") triggerHqPing(false);
        else if (act === "detail") {
          state.tab = "runs";
          embed.notifyRoute("/tako/win-lab/runs");
          loadRuns().then(function () {
            openRunDetail(btn.getAttribute("data-id"));
          });
        } else if (act === "close-detail") {
          state.selectedRun = null;
          render();
        } else if (act === "report-run") {
          reportRunToHq(btn.getAttribute("data-id"));
        } else if (act === "run-custom") {
          const appEl = document.getElementById("cand-app");
          const verEl = document.getElementById("cand-ver");
          const urlEl = document.getElementById("cand-url");
          state.customApp = appEl ? appEl.value : "codex";
          state.customVer = verEl ? verEl.value.trim() : "";
          state.customUrl = urlEl ? urlEl.value.trim() : "";
          if (!state.customUrl) {
            setToast("请填写候选 Windows 安装包 URL", true);
            return;
          }
          triggerGate(state.customApp, {
            candidate_url: state.customUrl,
            version: state.customVer,
          });
        }
      });
    });
  }

  loadActiveTab();
  setInterval(function () {
    if (state.overview && state.overview.badge && state.overview.badge.running_jobs > 0) {
      loadActiveTab();
    }
  }, 5000);
})();
