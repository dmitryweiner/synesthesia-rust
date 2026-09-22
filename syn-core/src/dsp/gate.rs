//! Fade gate for a disabled generator: fade to silence, after which the
//! caller stops computing its samples at all. Ported from
//! `../synesthesia/src/dsp/gate.ts`.

use crate::BLOCK;

/// A full fade takes this many blocks (~13 ms at 48 kHz).
pub const FADE_BLOCKS: usize = 5;
pub const FADE_STEP: f32 = 1.0 / (FADE_BLOCKS * BLOCK) as f32;

/// Silence without computing: the fade is done and the generator is off.
pub fn gate_is_silent(fade: f32, enabled: bool) -> bool {
    fade == 0.0 && !enabled
}

/// Moves `fade` toward its target (on → 1, off → 0), multiplying the buffer.
/// Returns the new fade; the buffer is untouched when fade is at its target.
pub fn apply_gate(buf: &mut [f32], fade: f32, enabled: bool) -> f32 {
    let target = if enabled { 1.0 } else { 0.0 };
    if fade == target {
        return fade;
    }
    let d = if target > fade { FADE_STEP } else { -FADE_STEP };
    let mut f = fade;
    for x in buf.iter_mut() {
        f = (f + d).clamp(0.0, 1.0);
        *x *= f;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_in_over_five_blocks() {
        let mut fade = 0.0;
        for _ in 0..FADE_BLOCKS {
            let mut buf = [1.0f32; BLOCK];
            fade = apply_gate(&mut buf, fade, true);
        }
        // 640 steps of 1/640 in f32 land just short of 1; one more block
        // clamps, and from there the gate is a no-op.
        assert!(fade > 0.99, "{fade}");
        let mut buf = [1.0f32; BLOCK];
        fade = apply_gate(&mut buf, fade, true);
        assert_eq!(fade, 1.0);
        assert_eq!(apply_gate(&mut buf, fade, true), 1.0);
    }

    #[test]
    fn fade_out_reaches_silence() {
        let mut fade = 1.0;
        for _ in 0..=FADE_BLOCKS {
            let mut buf = [1.0f32; BLOCK];
            fade = apply_gate(&mut buf, fade, false);
        }
        assert_eq!(fade, 0.0);
        assert!(gate_is_silent(fade, false));
    }

    #[test]
    fn silent_only_when_off_and_faded() {
        assert!(gate_is_silent(0.0, false));
        assert!(!gate_is_silent(0.0, true));
        assert!(!gate_is_silent(0.2, false));
    }
}
