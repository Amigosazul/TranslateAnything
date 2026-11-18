// 译文层的滚动跟随：实测位移 + 滚轮预测。
//
// 实测位移（后端截屏比对得出）总比界面晚一次截屏的时间，单靠它会"慢半拍"。
// 所以在框内滚动滚轮时，先按学到的"每格滚多少像素、滑多久"让译文立即滑起来，
// 同时用实测位移校正：实测跑到预测前面就跟实测；一次滚动结束后对齐到实测。
// 学习：每次滚动结束时用"实测总位移 / 滚轮格数"更新每格像素数，连续两次吻合才开始预测；
// 滚了滚轮但界面没动（到底了）就暂停该方向的预测。

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
/** 没有预测时，显示位置追赶实测位置的时间常数（毫秒） */
const FOLLOW_TAU = 22;

interface Gesture {
  /** 开始时的实测位置 */
  startM: number;
  /** 累计滚轮格数（向上为正） */
  notches: number;
  wheels: number;
  t0: number;
  lastWheel: number;
  /** 含非整格增量（精确式触控板）：直接跟手平移、没有滑动动画，不预测也不学习 */
  smooth: boolean;
  /** 实测位置的时间序列，用来学习滑动时长 */
  hist: [number, number][];
}

interface Prediction {
  from: number;
  target: number;
  t0: number;
  dur: number;
}

export class ScrollFollower {
  /** 实测累计滚动量、显示用的滚动量（物理像素） */
  measured: [number, number] = [0, 0];
  display: [number, number] = [0, 0];
  /** 每格滚轮对应的内容位移（像素，带符号）；null 表示还没学到 */
  ratio: number | null = null;
  samples = 0;
  /** 一格滚轮动画时长（毫秒） */
  duration = 160;
  /** 该方向滚不动了（1 向上 / -1 向下 / 0 无） */
  blockedDir = 0;

  private gesture: Gesture | null = null;
  private prediction: Prediction | null = null;
  private lastShiftAt = 0;
  private lastTick = 0;

  /** 几何变化或首次版面：直接对齐，丢掉进行中的滚动 */
  reset(total: [number, number]) {
    this.measured = [...total];
    this.display = [...total];
    this.gesture = null;
    this.prediction = null;
  }

  /** 后端推来的实测位移；age 为这帧画面截于多少毫秒前（按真实截屏时刻记录，用于学习滑动时长） */
  onMeasured(total: [number, number], now: number, age = 0) {
    this.measured = [...total];
    this.lastShiftAt = now;
    this.gesture?.hist.push([now - age, total[1]]);
  }

  /** 框内竖直滚轮（delta：一格 ±120，向上为正） */
  onWheel(delta: number, now: number) {
    const n = delta / 120;
    if (!this.gesture) {
      this.gesture = { startM: this.measured[1], notches: 0, wheels: 0, t0: now, lastWheel: now, smooth: false, hist: [[now, this.measured[1]]] };
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
    return (
      this.gesture !== null ||
      this.prediction !== null ||
      Math.abs(this.display[0] - this.measured[0]) > 0.3 ||
      Math.abs(this.display[1] - this.measured[1]) > 0.3
    );
  }

  /** 每帧调用，更新 display */
  tick(now: number) {
    const dt = this.lastTick ? Math.min(100, now - this.lastTick) : 16;
    this.lastTick = now;
    const g = this.gesture;
    const p = this.prediction;
    const [mx, my] = this.measured;

    if (g) {
      // 滚了滚轮但界面一直没动：到底了，取消预测
      if (p && now - g.t0 > NO_MOVE_CANCEL && Math.abs(my - g.startM) < 1) {
        this.blockedDir = Math.sign(g.notches);
        this.prediction = null;
      }
      if (now - g.lastWheel > GESTURE_IDLE_WHEEL && now - Math.max(this.lastShiftAt, g.t0) > GESTURE_IDLE_SHIFT) {
        this.finish(g);
      }
    }

    const k = 1 - Math.exp(-dt / FOLLOW_TAU);
    this.display[0] += (mx - this.display[0]) * k;
    const pred = this.prediction;
    if (pred && this.gesture) {
      const t = Math.min(1, (now - pred.t0) / pred.dur);
      const ease = 1 - Math.pow(1 - t, 3);
      let py = pred.from + (pred.target - pred.from) * ease;
      // 实测还没确认界面在动：限制领先量
      if (Math.abs(my - this.gesture.startM) < 1 && this.ratio !== null) {
        const cap = Math.max(UNCONFIRMED_LEAD, Math.abs(this.ratio) * UNCONFIRMED_RATIO);
        py = my + Math.max(-cap, Math.min(cap, py - my));
      }
      // 取预测与实测中"走得更远"的那个：实测追上来就跟实测
      const dir = Math.sign(pred.target - this.gesture.startM);
      this.display[1] = dir > 0 ? Math.max(py, my) : dir < 0 ? Math.min(py, my) : my;
    } else {
      this.display[1] += (my - this.display[1]) * k;
    }
    if (Math.abs(this.display[0] - mx) < 0.3) this.display[0] = mx;
    if (!this.prediction && Math.abs(this.display[1] - my) < 0.3) this.display[1] = my;
  }

  /** 一次滚动结束：学习每格像素数与滑动时长，之后对齐实测 */
  private finish(g: Gesture) {
    this.gesture = null;
    this.prediction = null;
    if (g.smooth || Math.abs(g.notches) < 0.5) return;
    const disp = this.measured[1] - g.startM;
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
