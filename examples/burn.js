/* This program burns as much CPU time as it can, while measuring runtime lag.
 *
 * CPU burn is achieved by computing random collatz trajectories.
 *
 * Runtime lag is measured by setTimeout() for a fixed interval of time (100ms),
 * and then measuring actual elapsed time when the timer finishes.
 * Doing so can measure various effects such as OS scheduling or event loop saturation / queue backloggin.
 *
 * We then report boxplot stats over a buffer of recent such timer measurements.
 * The buffer is sized so that it can contain samples from at least the last 10 seconds,
 * but when running into heavy delay, will cover a wider span of actual time.
 */

// @ts-check

/** @param {number} msec */
function sleep(msec) {
  return new Promise(resolve => setTimeout(resolve, msec));
}

function schedYield() {
  // @ts-ignore
  return new Promise(resolve => setImmediate(resolve));
}

/**
 * @param {object} config
 * @param {number} config.every -- how often to sample in msec
 * @param {number} config.keepLast -- timespan over which to keep data in msec
 */
function makeLagMonitor(config) {
  const { every, keepLast } = config;

  const bufferCap = Math.ceil(keepLast / every);
  const buffer = new Int32Array(bufferCap * 2);
  let bufferCur = 0;
  let bufferFull = false;

  /**
   * @param {number} t0
   * @param {number} t1
   */
  function collect(t0, t1) {
    buffer[bufferCur++] = t0;
    buffer[bufferCur++] = t1;
    if (bufferCur >= buffer.length) {
      bufferCur = bufferCur % buffer.length;
      bufferFull = true;
    }
  }

  let running = false;

  async function monitor() {
    let last = performance.now()
    for (running = true; running;) {
      await sleep(every);
      const now = performance.now()
      collect(last, now);
      last = now;
    }
  }

  return {
    start() {
      if (!running) monitor();
    },

    stop() {
      running = false;
    },

    *data() {
      const max = bufferFull ? buffer.length : bufferCur;
      for (let i = 0; i < max; i += 2) {
        let j = bufferFull ? (bufferCur + i) % buffer.length : i;
        const start = buffer[j];
        const end = buffer[j + 1];
        const actual = end - start;
        yield { start, end, actual, expected: every };
      }
    },

  };
}

function* burn() {
  // collatz recurrance goes brrr
  let n = Math.round(Math.random() * 1_000_000_000);
  while (n > 1) {
    yield n;
    n = n % 2 == 0 ? n / 2 : 3 * n + 1;
  }
}

async function burnForever() {
  let max = 0;
  /** @type {Array<number>} */
  let last = [];

  for (; ;) {

    // burn at least 2ms of elapsed time here before yielding back
    for (
      const start = performance.now();
      performance.now() - start < 2;
    ) {
      last = Array.from(burn());
      max = Math.max(max, last.length);
    }

    await schedYield();
  }
}

async function main() {
  const lagMonitor = makeLagMonitor({
    every: 100, // msec
    keepLast: 10_000, // msec
  });
  lagMonitor.start();

  burnForever();

  // Below we use boxplot stats to report on observed sleep time outliers
  //
  // See https://www.itl.nist.gov/div898/handbook/prc/section1/prc16.htm for maths refresher

  const threshold = 2; // minimum msec threshold to care about

  for (; ;) {
    await sleep(1_000);

    /** @type {Array<{start: number, actual: number, expected: number}>} */
    const sample = [];
    for (const { start, actual, expected } of lagMonitor.data()) {
      sample.push({ start, actual, expected });
    }
    sample.sort(({ actual: a }, { actual: b }) => a - b);

    const min = sample[0].actual;
    const max = sample[sample.length - 1].actual;
    if (max - min < threshold) {
      console.log(`[lag report] min:${min} max:${max}`);
      continue;
    }

    /** @param {number} q */
    const sampleQuantile = q => sample[Math.floor(q * sample.length)];

    const q25 = sampleQuantile(0.25).actual;
    const q50 = sampleQuantile(0.50).actual;
    const q75 = sampleQuantile(0.75).actual;
    const iqr = q75 - q25;

    const hi = q50 + Math.max(threshold, 3 * iqr); // "extreme" outliers only, mnot just "mile" ones at 1.5*
    const hiOutliers = sample.filter(({ actual }) => actual >= hi);

    if (hiOutliers.length == 0) {
      console.log(`[lag report] min:${min} max:${max} box:[ ${q25} ${q50} ${q75} ] no hi-outliers within threshold:${threshold}`);
      continue;
    }

    console.group(`[lag report] min:${min} max:${max} box:[ ${q25} ${q50} ${q75} ] hi:${hi} hiOutliers:${hiOutliers.length} ${Math.round(hiOutliers.length / sample.length * 1000) / 100}%`);
    console.table(hiOutliers);
    console.groupEnd();
  }

}

main();
