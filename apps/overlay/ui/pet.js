"use strict";

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
const pet = document.getElementById("pet");
const avatar = document.getElementById("pet-avatar");
const avatarImg = document.getElementById("pet-avatar-img");
const bubble = document.getElementById("bubble");
const fxLayer = document.getElementById("fx");
const skinMenu = document.getElementById("skin-menu");

// 浏览器预览回落：Tauri 后端不可用时仍能显示默认皮肤。
const FALLBACK = {
  images: {
    idle: "assets/skins/lingxi-hamster/idle.png",
    thinking: "assets/skins/lingxi-hamster/thinking.png",
    speaking: "assets/skins/lingxi-hamster/speaking.png",
    alert: "assets/skins/lingxi-hamster/alert.png",
  },
  anims: null,
  frame: null,
  bubbles: { idle: "灵犀", thinking: "思考中…", speaking: "建议好了", alert: "QQ 新消息" },
};

let config = FALLBACK;
let currentSkinId = "lingxi-hamster";
let status = "idle";
let down = null;
let lastPointer = null;
let dragDistance = 0;
let pettedThisDrag = false;

// ---- 帧动画驱动（petdex spritesheet：8 列，行数按素材规格动态探测）----
// 一个状态用 sheet 的某一行循环播放；rAF 按帧时长推进 background-position。
let animRun = null; // { sheet, row, frames, durationMs, dispW, dispH, rows, frameCols }
let animSheetLoaded = "";
let animRafId = 0;
let animEpoch = 0; // 状态/皮肤切换时作废进行中的循环

/// 帧动画慢放系数（1 = 按皮肤声明的原始节奏；2 = 整体慢一倍）。
/// 素材原始节奏偏快（眨眼过密），统一在这里调节。
const ANIM_SLOWDOWN = 2;

// petdex 素材有两种规格：v1 8×9（1536×1872）、v2 8×11（1536×2288），
// 帧尺寸都是 192×208。行数必须按图片实际高度算，写死 9 会把 v2 压扁
// 并在窗口底部漏出下一行的内容。
//
// 另外：素材每行末尾常有空白帧（画师按 8 列导出但只画了 5~7 帧），
// 按声明帧数播放就会周期性地"人物闪一下"。这里在加载后用 canvas 逐帧
// 采样 alpha，探测每行的**有效帧列索引**，播放时只走有效帧。
const sheetInfoCache = {}; // url -> { rows, validFrames: number[][] }
const sheetInfoPending = {};

function sheetInfo(url, frameW, frameH) {
  const hit = sheetInfoCache[url];
  if (hit) return hit;
  if (!sheetInfoPending[url]) {
    sheetInfoPending[url] = true;
    analyzeSheet(url, frameW, frameH)
      .then((info) => {
        if (!info) return;
        sheetInfoCache[url] = info;
        if (animRun && animRun.sheet === url) applySheetInfo(animRun, info);
      })
      .catch(() => {
        /* 分析失败：保持声明帧数的原行为 */
      });
  }
  return null;
}

/// 采样分析：返回每行的有效帧列索引；跨源污染（用户皮肤走 asset 协议）时返回 null。
async function analyzeSheet(url, frameW, frameH) {
  const img = new Image();
  img.src = url;
  await img.decode();
  const cols = Math.max(1, Math.round(img.naturalWidth / frameW));
  const rows = Math.max(1, Math.round(img.naturalHeight / frameH));
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(img, 0, 0);
  let pixels;
  try {
    pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  } catch {
    return null; // tainted canvas：退回声明值
  }
  const width = canvas.width;
  const stepX = Math.max(1, Math.floor(frameW / 24));
  const stepY = Math.max(1, Math.floor(frameH / 24));
  const minHits = 12; // 非透明采样点下限（低于此视为空帧）
  const validFrames = [];
  for (let row = 0; row < rows; row++) {
    const valid = [];
    for (let col = 0; col < cols; col++) {
      const x0 = col * frameW;
      const y0 = row * frameH;
      let hits = 0;
      for (let y = 0; y < frameH && hits < minHits * 4; y += stepY) {
        for (let x = 0; x < frameW; x += stepX) {
          const alpha = pixels[((y0 + y) * width + (x0 + x)) * 4 + 3];
          if (alpha > 8) hits++;
        }
      }
      if (hits >= minHits) valid.push(col);
    }
    validFrames.push(valid);
  }
  return { rows, validFrames };
}

/// 分析结果就绪后套用到正在播放的动画（纠正行数与有效帧列）。
function applySheetInfo(run, info) {
  run.rows = info.rows;
  const valid = info.validFrames[run.row];
  if (valid && valid.length) run.frameCols = valid;
}

function stopAnim() {
  animRun = null;
  cancelAnimationFrame(animRafId);
  animRafId = 0;
}

function startAnim(a) {
  const frame = config.frame;
  if (!frame || !frame.width || !frame.height) return false;
  const dispW = 200; // 与 .avatar 宽一致
  const dispH = Math.round((dispW * frame.height) / frame.width);
  avatar.style.height = dispH + "px";
  if (animSheetLoaded !== a.sheet) {
    avatar.style.backgroundImage = `url("${a.sheet}")`;
    animSheetLoaded = a.sheet;
  }
  const row = a.row || 0;
  const info = sheetInfo(a.sheet, frame.width, frame.height);
  animRun = {
    sheet: a.sheet,
    row,
    frames: Math.max(1, a.frames || 1),
    cols: Math.max(1, a.cols || 8),
    durationMs: Math.max(80, a.durationMs || 900),
    dispW,
    dispH,
    // 声明值先兜底，分析完成后由 applySheetInfo 精确纠正行数与有效帧。
    rows: info && info.rows ? info.rows : Math.max(1, a.rows || 9),
    frameCols:
      info && info.validFrames[row] && info.validFrames[row].length
        ? info.validFrames[row]
        : null,
  };
  // 全新起点：避免换行时从上一循环的时间戳继续跳帧。
  animEpoch = performance.now();
  if (!animRafId) animRafId = requestAnimationFrame(animTick);
  return true;
}

function animTick(now) {
  animRafId = 0;
  if (animRun) {
    // 有效帧列表优先：跳过素材行里未作画的空白帧（否则每轮会闪一下）。
    const frameCols =
      animRun.frameCols && animRun.frameCols.length ? animRun.frameCols : null;
    const count = frameCols ? frameCols.length : animRun.frames;
    const per = (animRun.durationMs * ANIM_SLOWDOWN) / count;
    const elapsed = now - animEpoch;
    const step = Math.floor(elapsed / per) % count;
    const col = frameCols ? frameCols[step] : step;
    avatar.style.backgroundSize = `${animRun.dispW * animRun.cols}px ${animRun.dispH * animRun.rows}px`;
    avatar.style.backgroundPosition = `${-col * animRun.dispW}px ${-animRun.row * animRun.dispH}px`;
    animRafId = requestAnimationFrame(animTick);
  }
}

function render(next) {
  status = next || "idle";
  // 只切状态类，保留 dragging/dropped/antic 等临时动画类。
  for (const s of ["idle", "thinking", "speaking", "alert"]) {
    pet.classList.toggle(s, s === status);
  }
  // 帧动画皮肤优先于静态图皮肤。
  const anim = (config.anims && config.anims[status]) || null;
  if (anim && startAnim(anim)) {
    avatarImg.style.display = "none";
    avatar.classList.add("is-anim");
  } else {
    stopAnim();
    avatar.classList.remove("is-anim");
    avatar.style.backgroundImage = "";
    avatar.style.width = "";
    avatar.style.height = "";
    animSheetLoaded = "";
    avatarImg.style.display = "";
    const nextAvatar = (config.images && config.images[status]) || FALLBACK.images.idle;
    if (!avatarImg.src.endsWith(nextAvatar)) avatarImg.src = nextAvatar;
  }
  // 互动反应气泡优先：临时台词没说完前不被状态轮询覆盖。
  if (!sayActive) bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
}

function applyConfig(view) {
  if (!view || !view.skin || (!view.images && !view.anims)) return;
  config = {
    images: view.images || {},
    anims: view.anims || null,
    frame: view.skin.frame || null,
    bubbles: view.bubbles,
  };
  currentSkinId = view.skin.id;
  render(status);
}

// ---- 点击操作菜单（A8-2 引擎进度控制台）----
// 桌宠定位收窄为「快速控制工作台进度」：显示引擎侧正在跑的会话/阶段，
// 待审批时一键允许/拒绝（工作台里的审批卡会同步收尾），有回合时一键中止。
// 交互只有一条规则：**点桌宠（左键单击或右键）就弹出可选项**——
// 不再有「单击=X、双击=Y」的按键记忆负担，能做什么全由菜单按当前状态列出。

/// 最近一次 /activity 快照（轮询更新；菜单按需读取）。
let activitySnapshot = { active: [], pending_approvals: 0 };

const PHASE_LABEL = {
  starting: "准备中",
  thinking: "思考中",
  speaking: "回复中",
  tool: "执行工具",
  waiting_approval: "等你审批",
};

function closeMenu() {
  skinMenu.hidden = true;
  skinMenu.replaceChildren();
}

function placeMenu(x, y) {
  const rect = skinMenu.getBoundingClientRect();
  const left = Math.max(6, Math.min(x - rect.width / 2, 214 - rect.width));
  const top = Math.max(6, Math.min(y - 10, 254 - rect.height));
  skinMenu.style.left = left + "px";
  skinMenu.style.top = top + "px";
}

function menuButton(label, onClick, extraClass) {
  const item = document.createElement("button");
  item.type = "button";
  item.className = "skin-item" + (extraClass ? " " + extraClass : "");
  const text = document.createElement("span");
  text.textContent = label;
  item.append(text);
  item.addEventListener("click", async () => {
    closeMenu();
    try {
      await onClick();
    } catch {
      /* 单项失败不打断菜单 */
    }
  });
  return item;
}

function openMenu(x, y) {
  skinMenu.replaceChildren();
  const active = (activitySnapshot.active || [])[0];
  // 信息行：当前正在跑的会话与阶段（进度控制台的核心读数）。
  const head = document.createElement("div");
  head.className = "menu-head";
  const pendingCount = activitySnapshot.pending_approvals || 0;
  head.textContent = active
    ? `${active.title || "会话"} · ${PHASE_LABEL[active.phase] || active.phase || ""}${
        active.tool ? " · " + active.tool : ""
      }`
    : pendingCount
      ? `待审批 ${pendingCount} 项`
      : "引擎空闲";
  skinMenu.appendChild(head);

  // 待审批：允许 / 拒绝（直连引擎；结果即时回执到所有客户端）。
  const waiting = (activitySnapshot.active || []).find(
    (item) => item.phase === "waiting_approval" && item.request_id
  );
  if (waiting && invoke) {
    skinMenu.appendChild(
      menuButton(
        `允许：${waiting.tool || "操作"}`,
        async () => {
          try {
            await invoke("owo_permission", {
              sessionId: waiting.session_id,
              requestId: waiting.request_id,
              allow: true,
            });
            sayTemp("已允许");
          } catch {
            sayTemp("审批失败");
          }
        },
        "is-allow"
      )
    );
    skinMenu.appendChild(
      menuButton(
        `拒绝：${waiting.tool || "操作"}`,
        async () => {
          try {
            await invoke("owo_permission", {
              sessionId: waiting.session_id,
              requestId: waiting.request_id,
              allow: false,
            });
            sayTemp("已拒绝");
          } catch {
            sayTemp("审批失败");
          }
        },
        "is-deny"
      )
    );
  }
  // 有活跃回合：中止。
  if (active && invoke) {
    skinMenu.appendChild(
      menuButton("中止当前任务", async () => {
        try {
          await invoke("owo_abort", { sessionId: active.session_id });
          sayTemp("已请求中止");
        } catch {
          sayTemp("中止失败");
        }
      })
    );
  }
  // 常驻项。
  if (invoke) {
    skinMenu.appendChild(
      menuButton("打开工作台", () => invoke("open_workbench"))
    );
    skinMenu.appendChild(
      menuButton("隐藏桌宠", () => invoke("set_pet_visible", { visible: false }))
    );
  }
  skinMenu.hidden = false;
  placeMenu(x, y);
}

pet.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  if (!skinMenu.hidden) {
    closeMenu();
    return;
  }
  openMenu(event.clientX, event.clientY);
});

document.addEventListener("mousedown", (event) => {
  if (!skinMenu.hidden && !skinMenu.contains(event.target)) closeMenu();
});

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !skinMenu.hidden) closeMenu();
});

// ---- 单击 / 拖动 ----
// 拖动带物理感：抓起时轻微倾斜缩放，松手时 squash & stretch 弹跳。
// 拖动用 Pointer Events + setPointerCapture：捕获后即使指针移出桌宠
// 元素（窗口移动滞后于鼠标时的常见情况），move/up 仍持续送达，
// 不会中途断流。这是“抚摸加了之后拖不动”的根治方案。

pet.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  try {
    pet.setPointerCapture(event.pointerId);
  } catch {
    /* capture 失败不影响拖动本身 */
  }
  down = { x: event.screenX, y: event.screenY };
  lastPointer = { x: event.screenX, y: event.screenY };
  dragDistance = 0;
  pettedThisDrag = false;
  pet.classList.add("dragging");
  pet.classList.remove("dropped");
});

pet.addEventListener("pointermove", (event) => {
  if (!down || event.buttons !== 1) return;
  const dx = event.screenX - lastPointer.x;
  const dy = event.screenY - lastPointer.y;
  lastPointer = { x: event.screenX, y: event.screenY };
  const step = Math.hypot(dx, dy);
  dragDistance += step;
  // 5px 死区：单击的微小抖动不应推动窗口。
  const total = Math.hypot(event.screenX - down.x, event.screenY - down.y);
  if (total > 5 && (dx || dy) && invoke) {
    // screenX/screenY 是 CSS 像素，而 move_pet_by 按物理像素移动窗口。
    // Windows 显示缩放 125%/150% 时两者差 devicePixelRatio 倍，不修正
    // 就会出现"桌宠追不上鼠标、像被粘住"的拖不动现象。
    const dpr = window.devicePixelRatio || 1;
    invoke("move_pet_by", {
      dx: Math.round(dx * dpr),
      dy: Math.round(dy * dpr),
    }).catch(() => {});
  }
  // 拖得够远就开心一下（摸摸头与拖动共存）。
  if (!pettedThisDrag && dragDistance > 420 && Date.now() > petCooldown) {
    pettedThisDrag = true;
    petCooldown = Date.now() + 3200;
    petted();
  }
});

function endDrag(event, movedOverride) {
  if (!down) return;
  const moved =
    movedOverride === true ||
    Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5;
  try {
    pet.releasePointerCapture(event.pointerId);
  } catch {
    /* already released */
  }
  down = null;
  lastPointer = null;
  pet.classList.remove("dragging");
  if (moved) {
    pet.classList.add("dropped");
    setTimeout(() => pet.classList.remove("dropped"), 620);
    return;
  }
  // 单击（非拖动）直接弹操作菜单——能做的事都在菜单里按当前状态列出；
  // 菜单开着时再点一下 = 关闭（与右键行为一致）。
  if (!skinMenu.hidden) {
    closeMenu();
    return;
  }
  openMenu(event.clientX, event.clientY);
}

pet.addEventListener("pointerup", (event) => endDrag(event));
pet.addEventListener("pointercancel", (event) => endDrag(event, true));

// 兜底：指针彻底丢失（设备拔出等）时复位拖拽态。
window.addEventListener("pointerup", () => {
  if (down) {
    down = null;
    lastPointer = null;
    pet.classList.remove("dragging");
  }
});

// ---- idle 彩蛋：偶尔左右张望，让形象更“活” ----
let anticTimer = 0;
function scheduleAntic() {
  clearTimeout(anticTimer);
  anticTimer = setTimeout(() => {
    if (status === "idle" && !down) {
      pet.classList.add("antic");
      setTimeout(() => pet.classList.remove("antic"), 1700);
    }
    scheduleAntic();
  }, 9000 + Math.random() * 15000);
}
scheduleAntic();

// ---- 互动：拖动摸摸头（拖得够远自然触发，无按键记忆负担）----

const PET_LINES = {
  "petdex-nailong": ["duang～再摸摸", "肚肚不许戳！", "龙龙很满意", "嘿嘿嘿"],
  "petdex-coco": ["呼噜呼噜…", "下巴这边再挠挠", "尾巴不许拽！", "喵呜～好舒服"],
  "petdex-mika": ["摸头会长不高的！", "哎呀～", "再摸要生气了哦", "诶嘿嘿"],
  "lingxi-hamster": ["吱！别摸啦", "再摸要打滚了", "嘿嘿…"],
  "lingxi-cat": ["喵～下巴这边", "呼噜噜…", "尾巴不许碰！"],
};
const PET_FALLBACK_LINES = ["嘿嘿，好痒～", "再摸摸我嘛", "(*´▽`*)"];

let petCooldown = 0;
let sayTimer = 0;
let sayActive = false;

function pick(list) {
  return list[Math.floor(Math.random() * list.length)];
}

// 临时说一句，几秒后回到当前状态的气泡。
function sayTemp(text, ms) {
  clearTimeout(sayTimer);
  sayActive = true;
  bubble.textContent = text;
  sayTimer = setTimeout(() => {
    sayActive = false;
    bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
  }, ms || 2200);
}

function spawnFx(glyph, count) {
  for (let i = 0; i < count; i++) {
    const s = document.createElement("span");
    s.className = "fx";
    s.textContent = glyph;
    s.style.left = 40 + Math.floor(Math.random() * 120) + "px";
    s.style.top = 55 + Math.floor(Math.random() * 60) + "px";
    s.style.animationDelay = (Math.random() * 0.3).toFixed(2) + "s";
    s.style.setProperty("--fx-rot", Math.floor(Math.random() * 50 - 25) + "deg");
    fxLayer.appendChild(s);
    setTimeout(() => s.remove(), 1700);
  }
}

function petted() {
  pet.classList.add("happy");
  setTimeout(() => pet.classList.remove("happy"), 1100);
  spawnFx("❤", 4);
  const lines = PET_LINES[currentSkinId] || PET_FALLBACK_LINES;
  sayTemp(pick(lines));
}

/// A8-2：轮询引擎活跃回合快照——桌宠的进度/审批都来自这里
/// （引擎空闲或未启动时静默；气泡回落事件驱动的本地状态）。
async function pollActivity() {
  if (!invoke) return;
  // A8-3：桌宠显隐双向同步（工作台开关 → 桌宠；本机切换 → 上报引擎）。
  try {
    await invoke("owo_pet_sync");
  } catch {
    /* 引擎未启动：保持本机状态 */
  }
  try {
    const data = await invoke("owo_activity");
    activitySnapshot = {
      active: Array.isArray(data && data.active) ? data.active : [],
      pending_approvals: Number((data && data.pending_approvals) || 0),
    };
    const head = (activitySnapshot.active || [])[0];
    if (!head) return;
    // 气泡跟随引擎侧进度（工作台/面板的回合都会出现在这里）。
    if (!sayActive) {
      const label = PHASE_LABEL[head.phase] || head.phase || "运行中";
      bubble.textContent = head.tool ? `${label}：${head.tool}` : label;
    }
    if (head.phase === "waiting_approval") render("alert");
    else if (head.phase === "speaking" || head.phase === "tool") render("speaking");
    else render("thinking");
  } catch {
    /* 引擎未启动/短暂不可达：保持现状 */
  }
}

render("idle");
if (invoke) {
  (async () => {
    try {
      applyConfig(await invoke("current_pet_config"));
    } catch {
      /* 后端未就绪时保持默认皮肤 */
    }
    // 窗口重建时读一次兜底：状态由 Rust 侧统一仲裁广播（M1），不再轮询 pet_status。
    try {
      render(await invoke("pet_status"));
    } catch {
      /* 后端未就绪时保持 idle */
    }
  })();
  pollActivity();
  setInterval(pollActivity, 2500);
}
if (listen) {
  listen("pet-config-changed", (event) => applyConfig(event.payload)).catch(() => {});
  // M1：Rust 侧仲裁后的桌宠状态（thinking / speaking / alert / idle）。
  listen("owo://pet-status", (event) => {
    const next = event && event.payload ? event.payload.status : "";
    if (typeof next === "string" && next) render(next);
  }).catch(() => {});
}
