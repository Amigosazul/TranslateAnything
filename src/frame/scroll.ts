// 译文层的滚动跟随：实测位移 + 速度外推 + 滚轮预测 + 不确定时淡出。
//
// 实测位移（后端截屏比对得出）总比界面晚一次截屏的时间，单靠它会"慢半拍"。
// - 速度外推：用最近两次实测（按真实截屏时刻）算出滚动速度，把译文推到"上屏那一刻"内容应在的位置；
//   测出在减速时速度按估出的时间常数衰减，最多外推 90ms，界面停下时外推随之收回。触控板、拖滚动条、翻页键都适用。
// - 滚轮预测：在框内滚动滚轮时，按学到的"每格滚多少像素、滑多久"让译文立即滑起来，不等第一次实测；
//   与外推位置取走得更远的那个。学习：每次滚动结束时用"实测总位移 / 滚轮格数"更新每格像素数，
//   连续两次吻合才开始预测；滚了滚轮但界面没动（到底了）就暂停该方向的预测。
// - 淡出：每次实测到达时，对比"截屏那一刻屏幕上译文的位置"与实测位置，得出真实错位量；
//   错位大时译文层变淡，稳定后淡回，避免译文压在错的行上。

/** 一次滚动结束的判定：最后一次滚轮之后、最后一次实测位移之后各静止这么久（毫秒） */
const GESTURE_IDLE_WHEEL = 300;
const GESTURE_IDLE_SHIFT = 200;
/** 滚轮后这么久界面还没动，就认为到底了，取消预测（毫秒）：约等于截屏延迟 + 两次截屏间隔 */
const NO_MOVE_CANCEL = 110;
/** 实测确认界面开始移动之前，预测最多领先这么多（像素，或一格位移的这个比例，取大者），
 *  这样即使滚到底了也只会轻微晃一下 */
const UNCONFIRMED_LEAD = 24;
const UNCONFIRMED_RATIO = 0.6;
/** 两次学习结果相差不超过该比例才算吻合 */
const CONSISTENT = 0.35;

/** 本帧设定的位置大约多久后才真正显示到屏幕上（毫秒）：透明 WebView 窗口约两帧（实测） */
const RENDER_LATENCY = 32;
/** 没测出减速时外推速度的衰减时间常数（毫秒）：基本按匀速外推，只在很远处收一点 */
const LEAD_DECAY = 200;
/** 外推最多看多远（毫秒） */
const LEAD_HORIZON = 90;
/** 测速时间窗（毫秒）：至少这么长、且不短于两个实测间隔，免得整数像素和短间隔带来噪声 */
const SPEED_WINDOW = 30;
/** 两次实测相隔超过这么久就不再据此算速度（毫秒），视为从静止起步 */
const MAX_SAMPLE_GAP = 120;/** 超过"预计下一次实测时刻"这么久仍没有新实测，视为已停下，外推在随后这段时间内收回（毫秒） */
const STALE_GRACE = 25;
const STALE_FADE = 50;
/** 向前追赶、静止对齐的时间常数（毫秒） */
const AHEAD_TAU = 8;
const SETTLE_TAU = 14;

/** 错位量（CSS 像素）：低于 OK 不淡出，达到 BAD 降到最淡 */
const ERR_OK = 5;
const ERR_BAD = 20;
const MIN_OPACITY = 0.2;
/** 错位记录的衰减时间常数（毫秒）：决定稳定后多快淡回 */
const ERR_DECAY = 140;
/** 显示位置历史保留多久（毫秒），用于回查截屏那一刻屏幕上的译文位置 */
const HISTORY_MS = 400;

interface Gesture {
  /** 开始时的实测位置 */
  startM: number;
  /** 累计滚轮格数（向上为正） */
  notches: number;
  wheels: number;
  t0: number;
  lastWheel: number;
  /** 含非整格增量（精确式触控板）：不做滚轮预测也不学习，靠速度外推跟随 */
  smooth: boolean;
  /** 实测位置的时间序列（按截屏时刻），用来学习滑动时长 */
  hist: [number, number][];
}

interface Prediction {
  from: number;
  target: number;
  t0: number;
  dur: number;
}

/** 单个方向上的实测与速度外推 */
class Axis {
  measured = 0;
  /** 速度（像素/毫秒） */
  v = 0;
  /** 最近一次实测的截屏时刻 */
  lastT = -Infinity;
  /** 最近一步位移（像素） */
  lastStep = 0;
  /** 相邻两次实测的典型间隔（毫秒） */
  interval = 33;
  /** 速度衰减时间常数（毫秒）：由相邻两个时间窗的速度比估出，减速越快越小 */
  tau = LEAD_DECAY;
  /** 最近一次位置真的变了的时刻 */
  lastMoveT = -Infinity;
  /** 最近的实测 [截屏时刻, 位置]，用来在时间窗内测速 */
  private hist: [number, number][] = [];

  reset(p: number) {
    this.measured = p;
    this.v = 0;
    this.lastT = -Infinity;
    this.lastStep = 0;
    this.lastMoveT = -Infinity;
    this.hist = [];
  }

  /** 某时刻的位置（在实测之间线性插值，早于记录时取最早一条） */
  private posAt(t: number) {
    const h = this.hist;
    if (t <= h[0][0]) return h[0][1];
    for (let i = h.length - 1; i > 0; i--) {
      if (h[i - 1][0] <= t) {
        const [ta, pa] = h[i - 1];
        const [tb, pb] = h[i];
        return tb > ta ? pa + ((pb - pa) * (t - ta)) / (tb - ta) : pb;
      }
    }
    return h[h.length - 1][1];
  }

  /** 新的实测：t 为截屏时刻 */
  onSample(p: number, t: number) {
    const gap = t - this.lastT;
    const step = p - this.measured;
    let onset = false;
    if (gap > 0 && gap <= MAX_SAMPLE_GAP) {
      this.interval = this.interval * 0.7 + Math.min(80, Math.max(5, gap)) * 0.3;
    }
    if (!(gap > 0 && gap <= MAX_SAMPLE_GAP) || this.lastStep === 0) {
      // 从静止起步：可能是平滑滚动的开头，也可能是没有动画的一次跳格，
      // 单凭一次实测分不清，先只对齐不外推，等第二次实测再测速
      this.hist = [[t - this.interval, this.measured]];
      onset = true;
    }
    this.hist.push([t, p]);
    while (this.hist.length > 2 && t - this.hist[0][0] > 200) this.hist.shift();

    // 最近一个时间窗的平均速度；再与前一个时间窗比较，减速时估出衰减时间常数，推算到本次截屏时刻
    const w = Math.max(SPEED_WINDOW, this.interval * 2);
    const start = Math.max(this.hist[0][0], t - w);
    const span = t - start;
    let v = span > 0 ? (p - this.posAt(start)) / span : 0;
    this.tau = LEAD_DECAY;
    if (onset) {
      v = 0;
    } else if (t - this.hist[0][0] >= 2 * w) {
      const v0 = (this.posAt(t - w) - this.posAt(t - 2 * w)) / w;
      const r = v0 !== 0 && Math.sign(v) === Math.sign(v0) ? v / v0 : 0;
      if (r > 0 && r < 0.95) {
        const tau = -w / Math.log(r);
        this.tau = Math.min(LEAD_DECAY, Math.max(8, tau));
        v *= Math.exp(-(w / 2) / this.tau);
      }
    }
    // 位置没变：后端只在画面静止一小段后才补发这种实测，说明已停下，不再外推
    if (step === 0) v = 0;
    this.v = v;
    if (step !== 0) this.lastMoveT = t;
    this.lastStep = step;
    this.measured = p;
    this.lastT = t;
  }

  /** 还在外推（最近有运动且没过期） */
  moving(now: number) {
    return this.v !== 0 && now - this.lastT < this.interval + STALE_GRACE + STALE_FADE;
  }

  /** 预计 now + RENDER_LATENCY 时内容所在的位置 */
  predict(now: number) {
    if (this.v === 0 || !isFinite(this.lastT)) return this.measured;
    const since = now - this.lastT;
    // 过了预计下一次实测的时刻还没新实测：多半停了，外推逐渐收回
    const stale = since - this.interval - STALE_GRACE;
    const keep = stale <= 0 ? 1 : Math.max(0, 1 - stale / STALE_FADE);
    if (keep === 0) return this.measured;
    const dt = Math.min(LEAD_HORIZON, since + RENDER_LATENCY);
    const lead = this.v * this.tau * (1 - Math.exp(-dt / this.tau));
    return this.measured + lead * keep;
  }
}

export class ScrollFollower {
  /** 显示用的滚动量（物理像素） */
  display: [number, number] = [0, 0];
  /** 译文层不透明度：错位大时变淡 */
  opacity = 1;
  /** 设备像素比：错位阈值按 CSS 像素给出 */
  scale = 1;
  /** 每格滚轮对应的内容位移（像素，带符号）；null 表示还没学到 */
  ratio: number | null = null;
  samples = 0;
  /** 一格滚轮动画时长（毫秒） */
  duration = 160;
  /** 该方向滚不动了（1 向上 / -1 向下 / 0 无） */
  blockedDir = 0;

  private axes = [new Axis(), new Axis()];
  private gesture: Gesture | null = null;
  private prediction: Prediction | null = null;
  private lastShiftAt = 0;
  private lastTick = 0;
  /** 最近观测到的错位量（物理像素），随时间衰减 */
  private err = 0;
  /** 显示位置历史 [时刻, x, y] */
  private history: [number, number, number][] = [];

  /** 实测累计滚动量（物理像素） */
  get measured(): [number, number] {
    return [this.axes[0].measured, this.axes[1].measured];
  }

  /** 几何变化或首次版面：直接对齐，丢掉进行中的滚动 */
  reset(total: [number, number]) {
    this.axes[0].reset(total[0]);
    this.axes[1].reset(total[1]);
    this.display = [...total];
    this.gesture = null;
    this.prediction = null;
    this.err = 0;
    this.opacity = 1;
    this.history = [];
  }

  /** 版面消息带来的累计滚动量：只在与实测不一致时校正，不参与测速 */
  sync(total: [number, number], now: number) {
    for (let i = 0; i < 2; i++) {
      const a = this.axes[i];
      if (a.measured !== total[i]) {
        a.measured = total[i];
        a.v = 0;
        a.lastT = now;
      }
    }
  }

  /** 后端推来的实测位移；age 为这帧画面截于多少毫秒前 */
  onMeasured(total: [number, number], now: number, age = 0) {
    const t = now - age;
    for (let i = 0; i < 2; i++) {
      const a = this.axes[i];
      // 错位量：截屏那一刻屏幕上的译文位置 vs 实测位置。从静止起步的第一次实测必然落后，不计
      const wasTracking = a.moving(now) || (i === 1 && this.prediction !== null);
      if (wasTracking) {
        const shown = this.shownAt(t - RENDER_LATENCY, i);
        if (shown !== null) this.err = Math.max(this.err, Math.abs(shown - total[i]));
      }
      const prevStep = a.lastStep;
      a.onSample(total[i], t);
      // 预测的滑动动画本该还在进行，实测却停了：滚到头了，立即取消预测，不必等 NO_MOVE_CANCEL
      const p = this.prediction;
      if (i === 1 && p && this.gesture && prevStep !== 0 && a.lastStep === 0 && t < p.t0 + p.dur && Math.abs(p.target - total[1]) > 2) {
        this.blockedDir = Math.sign(this.gesture.notches);
        this.prediction = null;
      }
    }
    this.lastShiftAt = now;
    this.gesture?.hist.push([t, total[1]]);
  }

  /** 框内竖直滚轮（delta：一格 ±120，向上为正） */
  onWheel(delta: number, now: number) {
    const n = delta / 120;
    const my = this.axes[1].measured;
    if (!this.gesture) {
      this.gesture = { startM: my, notches: 0, wheels: 0, t0: now, lastWheel: now, smooth: false, hist: [[now, my]] };
    }
    const g = this.gesture;
    if (delta % 120 !== 0) {
      g.smooth = true;
      this.prediction = null;
    }
    g.notches += n;
    g.wheels += 1;
    g.lastWheel = now;
    if (!g.smooth && this.ratio !== null && this.samples >= 2 && Math.sign(n) !== this.blockedDir) {
      this.prediction = {
        from: this.display[1],
        target: g.startM + this.ratio * g.notches,
        t0: now,
        dur: this.duration,
      };
    }
  }

  /** 是否还需要继续逐帧刷新 */
  get active() {
    const now = this.lastTick;
    return (
      this.gesture !== null ||
      this.prediction !== null ||
      this.opacity < 1 ||
      this.axes.some((a, i) => a.moving(now) || Math.abs(this.display[i] - a.measured) > 0.3)
    );
  }

  /** 每帧调用，更新 display 与 opacity */
  tick(now: number) {
    const dt = this.lastTick ? Math.min(100, Math.max(0, now - this.lastTick)) : 16;
    this.lastTick = now;
    const g = this.gesture;
    const my = this.axes[1].measured;

    if (g) {
      // 滚了滚轮但界面一直没动：到底了，取消预测
      if (this.prediction && now - g.t0 > NO_MOVE_CANCEL && Math.abs(my - g.startM) < 1) {
        this.blockedDir = Math.sign(g.notches);
        this.prediction = null;
      }
      // 动了一段又停在预测目标之前：滚到头了，同样取消预测，之后这个方向先不预测
      const p = this.prediction;
      if (
        p &&
        now - Math.max(this.axes[1].lastMoveT, p.t0) > NO_MOVE_CANCEL &&
        Math.abs(p.target - my) > 2
      ) {
        this.blockedDir = Math.sign(g.notches);
        this.prediction = null;
      }
      if (now - g.lastWheel > GESTURE_IDLE_WHEEL && now - Math.max(this.lastShiftAt, g.t0) > GESTURE_IDLE_SHIFT) {
        this.finish(g);
      }
    }

    for (let i = 0; i < 2; i++) {
      const a = this.axes[i];
      let target = a.predict(now);
      let dir = Math.sign(a.v);
      const p = i === 1 ? this.prediction : null;
      if (p && this.gesture) {
        const t = Math.min(1, (now + RENDER_LATENCY - p.t0) / p.dur);
        const ease = 1 - Math.pow(1 - t, 3);
        let py = p.from + (p.target - p.from) * ease;
        // 实测还没确认界面在动：限制领先量
        if (Math.abs(my - this.gesture.startM) < 1 && this.ratio !== null) {
          const cap = Math.max(UNCONFIRMED_LEAD, Math.abs(this.ratio) * UNCONFIRMED_RATIO);
          py = my + Math.max(-cap, Math.min(cap, py - my));
        }
        // 取预测与外推中"走得更远"的那个：实测追上来就跟实测。
        // 外推不越过预测终点（除非实测本身已经越过）：缓出到末段时外推容易冲过头
        dir = Math.sign(p.target - this.gesture.startM);
        if (dir > 0) target = Math.max(py, Math.min(target, Math.max(p.target, my)));
        else if (dir < 0) target = Math.min(py, Math.max(target, Math.min(p.target, my)));
      }
      const animating = p !== null && now < p.t0 + p.dur;
      const moving = a.moving(now) || animating;
      this.display[i] = this.follow(this.display[i], target, dir, moving, dt);
      if (!moving && Math.abs(this.display[i] - a.measured) < 0.3) this.display[i] = a.measured;
    }

    this.history.push([now, this.display[0], this.display[1]]);
    while (this.history.length > 2 && now - this.history[0][0] > HISTORY_MS) this.history.shift();

    // 错位量衰减 → 不透明度
    this.err *= Math.exp(-dt / ERR_DECAY);
    if (this.err < 0.3) this.err = 0;
    const e = this.err / this.scale;
    const k = Math.min(1, Math.max(0, (e - ERR_OK) / (ERR_BAD - ERR_OK)));
    this.opacity = 1 - k * (1 - MIN_OPACITY);
  }

  /** 显示位置朝目标推进：顺着滚动方向几乎立即跟上；滚动途中目标往回缩（外推冲过头、估出在减速）
   *  时原地等内容追上，不往回拉——内容还在往前走，回拉只会来回抖；停下后再对齐实测 */
  private follow(cur: number, target: number, dir: number, moving: boolean, dt: number) {
    const diff = target - cur;
    if (!moving) return cur + diff * (1 - Math.exp(-dt / SETTLE_TAU));
    if (dir !== 0 && Math.sign(diff) === -dir) return cur;
    return cur + diff * (1 - Math.exp(-dt / AHEAD_TAU));
  }

  /** 某时刻设定的显示位置（在历史中插值）；没有记录时返回 null */
  private shownAt(t: number, i: number): number | null {
    const h = this.history;
    if (h.length === 0 || t < h[0][0]) return null;
    for (let k = h.length - 1; k >= 0; k--) {
      if (h[k][0] <= t) {
        const a = h[k];
        const b = h[k + 1];
        if (!b) return a[i + 1];
        const f = (t - a[0]) / Math.max(1e-6, b[0] - a[0]);
        return a[i + 1] + (b[i + 1] - a[i + 1]) * f;
      }
    }
    return null;
  }

  /** 一次滚动结束：学习每格像素数与滑动时长 */
  private finish(g: Gesture) {
    this.gesture = null;
    this.prediction = null;
    if (g.smooth || Math.abs(g.notches) < 0.5) return;
    const disp = this.axes[1].measured - g.startM;
    if (Math.abs(disp) < 2) {
      this.blockedDir = Math.sign(g.notches);
      return;
    }
    this.blockedDir = 0;
    const sample = disp / g.notches;
    if (this.ratio !== null && Math.sign(sample) === Math.sign(this.ratio) && Math.abs(sample - this.ratio) <= CONSISTENT * Math.abs(this.ratio)) {
      this.ratio = this.ratio * 0.6 + sample * 0.4;
      this.samples += 1;
    } else {
      this.ratio = sample;
      this.samples = 1;
    }
    // 只有一格的滚动才能干净地量出滑动时长：到达 90% 位移的时刻。
    // 按缓出曲线 1-(1-t)^3，到 90% 时 t ≈ 0.536
    if (g.wheels === 1) {
      const goal = Math.abs(disp) * 0.9;
      const i = g.hist.findIndex(([, m]) => Math.abs(m - g.startM) >= goal);
      if (i > 0) {
        // 在相邻两个实测点之间线性插值，求到达 90% 的时刻
        const [ta, ma] = g.hist[i - 1];
        const [tb, mb] = g.hist[i];
        const da = Math.abs(ma - g.startM);
        const db = Math.abs(mb - g.startM);
        const t90 = db > da ? ta + ((goal - da) / (db - da)) * (tb - ta) : tb;
        const d = Math.min(400, Math.max(60, (t90 - g.t0) / 0.536));
        this.duration = this.duration * 0.5 + d * 0.5;
      }
    }
  }
}
