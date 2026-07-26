//! CNN digit classifier — hand-written forward pass.
//!
//! Architecture: 3×Conv(3×3,pad=1)→ReLU→MaxPool(2×2) → FC→ReLU → FC → Softmax
//! Conv-BN layers are pre-fused at export time (Python), so only Conv weights
//! need to be applied here.
//!
//! Input:  24×24 grayscale patch (576 f32, row-major, values 0.0–1.0)
//! Output: (class 0-9 or 10=non-digit, confidence 0.0–1.0)
//!
//! Weights embedded via include_bytes!("../resources/cnn_weights.bin").
//! Total: 20,299 f32 = 81,196 bytes.  ~20K MAC per classify → <0.1 ms.

// ── Layer dimensions ────────────────────────────────────────────────────
const C1_IN: usize = 1;
const C1_OUT: usize = 8;
const C1_K: usize = 3;
const C1_W: usize = C1_OUT * C1_IN * C1_K * C1_K; // 72
const C1_B: usize = C1_OUT; // 8

const C2_IN: usize = 8;
const C2_OUT: usize = 16;
const C2_K: usize = 3;
const C2_W: usize = C2_OUT * C2_IN * C2_K * C2_K; // 1152
const C2_B: usize = C2_OUT; // 16

const C3_IN: usize = 16;
const C3_OUT: usize = 32;
const C3_K: usize = 3;
const C3_W: usize = C3_OUT * C3_IN * C3_K * C3_K; // 4608
const C3_B: usize = C3_OUT; // 32

const FC1_IN: usize = 288; // 32 × 3 × 3
const FC1_OUT: usize = 48;
const FC1_W: usize = FC1_OUT * FC1_IN; // 13824
const FC1_B: usize = FC1_OUT; // 48

const FC2_IN: usize = 48;
const FC2_OUT: usize = 11;
const FC2_W: usize = FC2_OUT * FC2_IN; // 528
const FC2_B: usize = FC2_OUT; // 11

const INPUT_H: usize = 24;
const INPUT_W: usize = 24;
const INPUT_LEN: usize = INPUT_H * INPUT_W; // 576

const BLOB_LEN: usize = (C1_W + C1_B + C2_W + C2_B + C3_W + C3_B
    + FC1_W + FC1_B + FC2_W + FC2_B)
    * 4; // 81,196

// ── Cnn struct ──────────────────────────────────────────────────────────

pub struct Cnn {
    conv1_w: [f32; C1_W],
    conv1_b: [f32; C1_B],
    conv2_w: [f32; C2_W],
    conv2_b: [f32; C2_B],
    conv3_w: [f32; C3_W],
    conv3_b: [f32; C3_B],
    fc1_w: [f32; FC1_W],
    fc1_b: [f32; FC1_B],
    fc2_w: [f32; FC2_W],
    fc2_b: [f32; FC2_B],
}

impl Cnn {
    /// Load weights from the embedded binary blob.
    /// Panics if the blob size doesn't match or data is misaligned.
    pub fn load() -> Self {
        let blob: &[u8] = include_bytes!("../resources/cnn_weights.bin");
        assert_eq!(
            blob.len(),
            BLOB_LEN,
            "cnn_weights.bin: expected {BLOB_LEN} bytes, got {}",
            blob.len()
        );

        // SAFETY: include_bytes! embeds data in the static section which is
        // 4-byte (or better) aligned on all tier-1 targets.  The blob length
        // is guaranteed to be a multiple of 4 (it's all f32s).
        let data: &[f32] =
            unsafe { std::slice::from_raw_parts(blob.as_ptr() as *const f32, blob.len() / 4) };

        let mut off = 0;
        let mut next = |n: usize| {
            let slice = &data[off..off + n];
            off += n;
            slice
        };

        let c1w_slice = next(C1_W);
        let c1b_slice = next(C1_B);
        let c2w_slice = next(C2_W);
        let c2b_slice = next(C2_B);
        let c3w_slice = next(C3_W);
        let c3b_slice = next(C3_B);
        let f1w_slice = next(FC1_W);
        let f1b_slice = next(FC1_B);
        let f2w_slice = next(FC2_W);
        let f2b_slice = next(FC2_B);
        debug_assert_eq!(off, data.len());

        // Copy slices into fixed-size arrays (the compiler should optimise
        // these into memcpy, but the load only runs once).
        let mut conv1_w = [0f32; C1_W];
        conv1_w.copy_from_slice(c1w_slice);
        let mut conv1_b = [0f32; C1_B];
        conv1_b.copy_from_slice(c1b_slice);
        let mut conv2_w = [0f32; C2_W];
        conv2_w.copy_from_slice(c2w_slice);
        let mut conv2_b = [0f32; C2_B];
        conv2_b.copy_from_slice(c2b_slice);
        let mut conv3_w = [0f32; C3_W];
        conv3_w.copy_from_slice(c3w_slice);
        let mut conv3_b = [0f32; C3_B];
        conv3_b.copy_from_slice(c3b_slice);
        let mut fc1_w = [0f32; FC1_W];
        fc1_w.copy_from_slice(f1w_slice);
        let mut fc1_b = [0f32; FC1_B];
        fc1_b.copy_from_slice(f1b_slice);
        let mut fc2_w = [0f32; FC2_W];
        fc2_w.copy_from_slice(f2w_slice);
        let mut fc2_b = [0f32; FC2_B];
        fc2_b.copy_from_slice(f2b_slice);

        Self {
            conv1_w,
            conv1_b,
            conv2_w,
            conv2_b,
            conv3_w,
            conv3_b,
            fc1_w,
            fc1_b,
            fc2_w,
            fc2_b,
        }
    }

    /// Classify a 24×24 grayscale patch (values 0.0–1.0, row-major).
    ///
    /// Returns `(class, confidence)` where class 0–9 = digit, 10 = non-digit.
    /// Confidence is the softmax probability of the winning class.
    pub fn classify(&self, patch: &[f32; INPUT_LEN]) -> (u8, f32) {
        // ── Conv1: 1→8, 3×3, pad=1 ──────────────────────────────────
        let mut c1 = [0f32; C1_OUT * INPUT_H * INPUT_W]; // 8×24×24
        conv2d_3x3_pad1::<C1_IN, C1_OUT, INPUT_H, INPUT_W>(
            patch, &mut c1, &self.conv1_w, &self.conv1_b,
        );
        relu_inplace(&mut c1);

        // ── Pool1: 8×24×24 → 8×12×12 ───────────────────────────────
        let mut p1 = [0f32; C1_OUT * 12 * 12]; // 1152
        maxpool2d_2x2::<C1_OUT, 24, 24>(&c1, &mut p1);

        // ── Conv2: 8→16, 3×3, pad=1 ────────────────────────────────
        let mut c2 = [0f32; C2_OUT * 12 * 12]; // 16×12×12
        conv2d_3x3_pad1::<C2_IN, C2_OUT, 12, 12>(
            &p1, &mut c2, &self.conv2_w, &self.conv2_b,
        );
        relu_inplace(&mut c2);

        // ── Pool2: 16×12×12 → 16×6×6 ───────────────────────────────
        let mut p2 = [0f32; C2_OUT * 6 * 6]; // 576
        maxpool2d_2x2::<C2_OUT, 12, 12>(&c2, &mut p2);

        // ── Conv3: 16→32, 3×3, pad=1 ───────────────────────────────
        let mut c3 = [0f32; C3_OUT * 6 * 6]; // 32×6×6
        conv2d_3x3_pad1::<C3_IN, C3_OUT, 6, 6>(
            &p2, &mut c3, &self.conv3_w, &self.conv3_b,
        );
        relu_inplace(&mut c3);

        // ── Pool3: 32×6×6 → 32×3×3 ─────────────────────────────────
        let mut p3 = [0f32; C3_OUT * 3 * 3]; // 288
        maxpool2d_2x2::<C3_OUT, 6, 6>(&c3, &mut p3);

        // ── FC1: 288 → 48 ──────────────────────────────────────────
        let mut fc1 = [0f32; FC1_OUT];
        linear::<FC1_IN, FC1_OUT>(&p3, &mut fc1, &self.fc1_w, &self.fc1_b);
        relu_inplace(&mut fc1);

        // ── FC2: 48 → 11 ───────────────────────────────────────────
        let mut fc2 = [0f32; FC2_OUT];
        linear::<FC2_IN, FC2_OUT>(&fc1, &mut fc2, &self.fc2_w, &self.fc2_b);

        // ── Softmax ────────────────────────────────────────────────
        softmax_inplace(&mut fc2);

        // ── Argmax ─────────────────────────────────────────────────
        let mut best_class = 0u8;
        let mut best_conf = fc2[0];
        for i in 1..FC2_OUT {
            if fc2[i] > best_conf {
                best_conf = fc2[i];
                best_class = i as u8;
            }
        }

        (best_class, best_conf)
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Primitives
// ══════════════════════════════════════════════════════════════════════════

/// 3×3 Conv2d with padding=1, stride=1.  Keeps spatial dims unchanged.
/// `input` / `output` are NCHW row-major flat arrays.
#[allow(clippy::needless_range_loop)]
fn conv2d_3x3_pad1<const IC: usize, const OC: usize, const H: usize, const W: usize>(
    input: &[f32],
    output: &mut [f32],
    weight: &[f32], // [OC][IC][3][3]
    bias: &[f32],   // [OC]
) {
    let kh = 3usize;
    let kw = 3usize;
    for oc in 0..OC {
        let b = bias[oc];
        for y in 0..H {
            for x in 0..W {
                let mut sum = b;
                for ic in 0..IC {
                    for dy in 0..kh {
                        let iy = y as isize + dy as isize - 1isize;
                        if iy < 0 || iy >= H as isize {
                            continue;
                        }
                        let iy = iy as usize;
                        for dx in 0..kw {
                            let ix = x as isize + dx as isize - 1isize;
                            if ix < 0 || ix >= W as isize {
                                continue;
                            }
                            let ix = ix as usize;
                            let w_idx = ((oc * IC + ic) * kh + dy) * kw + dx;
                            let i_idx = (ic * H + iy) * W + ix;
                            sum += weight[w_idx] * input[i_idx];
                        }
                    }
                }
                output[(oc * H + y) * W + x] = sum;
            }
        }
    }
}

/// 2×2 MaxPool with stride=2.
fn maxpool2d_2x2<const C: usize, const H: usize, const W: usize>(
    input: &[f32],    // [C][H][W]
    output: &mut [f32], // [C][H/2][W/2]
) {
    let oh = H / 2;
    let ow = W / 2;
    for c in 0..C {
        for oy in 0..oh {
            for ox in 0..ow {
                let iy = oy * 2;
                let ix = ox * 2;
                let base = (c * H + iy) * W + ix;
                let a = input[base];
                let b = input[base + 1];
                let c2 = input[base + W];
                let d = input[base + W + 1];
                let mut m = a;
                if b > m { m = b; }
                if c2 > m { m = c2; }
                if d > m { m = d; }
                output[(c * oh + oy) * ow + ox] = m;
            }
        }
    }
}

fn relu_inplace(x: &mut [f32]) {
    for v in x.iter_mut() {
        *v = v.max(0.0);
    }
}

fn linear<const IN: usize, const OUT: usize>(
    input: &[f32],   // [IN]
    output: &mut [f32], // [OUT]
    weight: &[f32],   // [OUT][IN]
    bias: &[f32],     // [OUT]
) {
    for o in 0..OUT {
        let mut sum = bias[o];
        let row_base = o * IN;
        for i in 0..IN {
            sum += weight[row_base + i] * input[i];
        }
        output[o] = sum;
    }
}

fn softmax_inplace(x: &mut [f32]) {
    let n = x.len();
    // Numerically stable: subtract max before exp
    let mut max_val = x[0];
    for i in 1..n {
        if x[i] > max_val {
            max_val = x[i];
        }
    }
    let mut sum = 0.0f32;
    for v in x.iter_mut() {
        *v = (*v - max_val).exp();
        sum += *v;
    }
    if sum > 0.0 {
        for v in x.iter_mut() {
            *v /= sum;
        }
    }
}
