//! The whole picture behind one pair of calls: [`Picture::step`] on the
//! simulation clock, [`Picture::draw`] on the redraw clock. It does what the
//! web app's frame loop does between the features and the screen — onset
//! hits become growth and ripples, the point becomes this frame's params —
//! and it is what both the live field thread and `render --picture` run.

use super::coupling::RippleSet;
use super::display::{render, Image};
use super::frame::{frame_params, FrameParams};
use super::{grid_for_pixels, Sim};
use crate::features::AudioFeatures;
use crate::sim::coupling::MAX_RIPPLES;
use crate::state::AppState;

pub struct Picture {
    sim: Sim,
    ripples: RippleSet,
    image: Image,
    frame: Option<FrameParams>,
    /// The engine's hit counter as last seen; `None` until the first step.
    hits: Option<u64>,
}

impl Picture {
    /// A picture of `w`×`h` pixels on the grid [`grid_for_pixels`] picks.
    pub fn new(w: usize, h: usize, seed: u32) -> Self {
        let (gw, gh) = grid_for_pixels(w, h);
        Self {
            sim: Sim::new(gw, gh, seed),
            ripples: RippleSet::default(),
            image: Image::new(w, h),
            frame: None,
            hits: None,
        }
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn reseed(&mut self) {
        self.sim.reseed();
    }

    /// One simulation step at engine time `t`: new hits since the last step
    /// (at most as many as there are ripples) seed growth, then the field
    /// advances under the point's params as the LFOs and the sound have them.
    pub fn step(&mut self, state: &AppState, features: &AudioFeatures, hits: u64, t: f64) {
        let new_hits = self.hits.map_or(0, |seen| hits.saturating_sub(seen)).min(MAX_RIPPLES as u64);
        self.hits = Some(hits);
        let amount = state.coupling.get("onsetToSeed").copied().unwrap_or(0.0);
        for _ in 0..new_hits {
            if let Some((x, y)) = self.sim.seed_on_hit(amount) {
                self.ripples.add(x, y, amount as f32, t);
            }
        }
        let frame = frame_params(state, features, t);
        self.sim.step(&frame.sim);
        self.frame = Some(frame);
    }

    /// The picture as of the last step, with the ripples alive at `t`.
    pub fn draw(&mut self, t: f64) -> &Image {
        if let Some(f) = self.frame {
            let ripples = self.ripples.active(t);
            render(self.sim.field(), &f.palette, &f.fx, &ripples, &mut self.image);
        }
        &self.image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::presets;

    #[test]
    fn hits_grow_the_field_and_ripple_the_picture() {
        let state = &presets()[0].state;
        let quiet = AudioFeatures { brightness: 0.5, ..Default::default() };
        let (mut a, mut b) = (Picture::new(24, 12, 4), Picture::new(24, 12, 4));
        for (i, p) in [&mut a, &mut b].into_iter().enumerate() {
            p.step(state, &quiet, 10, 0.0);
            // The first step only learns the counter; b then sees two hits.
            p.step(state, &quiet, 10 + 2 * i as u64, 1.0 / 30.0);
        }
        assert_ne!(a.sim().field().v(), b.sim().field().v());
        assert_ne!(a.draw(0.05), b.draw(0.05));
    }

    #[test]
    fn nothing_is_drawn_before_the_first_step() {
        let mut p = Picture::new(8, 4, 1);
        assert!(p.draw(0.0).rgb.iter().all(|&c| c == [0, 0, 0]));
    }
}
