//! One frame's inputs, derived the way the web app's frame loop derives
//! them (`loop()` in `src/main.ts`): card params through the LFOs, then
//! through the audio couplings; off Field variation / Flow cards count as
//! their zeros; the display effects from the features.

use std::collections::BTreeMap;

use super::coupling::{apply_coupling, display_coupling, DisplayFx};
use super::palette::Palette;
use super::{FieldVariation, Flow, Reaction, SimParams};
use crate::features::AudioFeatures;
use crate::modmatrix::effective_params;
use crate::schema::card_ranges;
use crate::state::{AppState, Params};

/// Every card's params with the mod matrix applied at time `t`.
pub fn effective_cards(state: &AppState, t: f64) -> BTreeMap<String, Params> {
    let m = &state.modulation;
    state
        .visual
        .cards
        .iter()
        .map(|(id, card)| {
            (id.clone(), effective_params(id, &card.params, &m.lfos, &m.routes, &card_ranges(id), t))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameParams {
    pub sim: SimParams,
    pub palette: Palette,
    pub fx: DisplayFx,
}

pub fn frame_params(state: &AppState, features: &AudioFeatures, t: f64) -> FrameParams {
    let eff = apply_coupling(&effective_cards(state, t), features, &state.coupling);
    let on = |id: &str| state.visual.cards.get(id).is_some_and(|c| c.on);
    let empty = Params::new();
    let card = |id: &str| eff.get(id).unwrap_or(&empty);
    FrameParams {
        sim: SimParams {
            reaction: Reaction::from_card(card("reaction")),
            field_variation: if on("fieldVariation") {
                FieldVariation::from_card(card("fieldVariation"))
            } else {
                FieldVariation::ZERO
            },
            flow: if on("flow") { Flow::from_card(card("flow")) } else { Flow::ZERO },
        },
        palette: Palette::from_card(card("palette")),
        fx: display_coupling(features, &state.coupling),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modmatrix::{LfoDef, LfoShape, ModRoute};
    use crate::state::presets;

    #[test]
    fn neutral_sound_and_no_routes_give_the_cards_as_they_are() {
        // The presets route LFOs onto visual params (Fractal garden sweeps
        // feed and curl), so the routes go first.
        for p in presets() {
            let mut s = p.state.clone();
            s.modulation.routes.clear();
            // Brightness at its midpoint: `brightToShift` is centred there, so
            // true silence (brightness 0, as in the web app's SILENT_FEATURES)
            // shifts the hue by half the gene — the browser does the same.
            let neutral = AudioFeatures { brightness: 0.5, ..Default::default() };
            let f = frame_params(&s, &neutral, 3.0);
            assert_eq!(f.sim, SimParams::from_cards(&s.visual.cards), "{}", p.name);
            assert_eq!(f.palette, Palette::from_card(&s.visual.cards["palette"].params));
        }
    }

    #[test]
    fn an_lfo_route_moves_a_visual_param() {
        let mut s = presets()[0].state.clone();
        s.modulation.routes.clear();
        s.modulation.lfos = vec![LfoDef { shape: LfoShape::Square, rate: 1.0, phase: 0.0 }];
        s.modulation.routes = vec![ModRoute {
            src: 0,
            target: "reaction".into(),
            param: "feed".into(),
            depth: 0.5,
            exp: false,
        }];
        let base = s.visual.cards["reaction"].params["feed"];
        let a = frame_params(&s, &AudioFeatures::default(), 0.1).sim.reaction.feed;
        let b = frame_params(&s, &AudioFeatures::default(), 0.6).sim.reaction.feed;
        assert!(a != b, "a square LFO flips between its halves");
        assert!((a - base).abs() > 1e-6 && (b - base).abs() > 1e-6);
    }

    #[test]
    fn loudness_reaches_the_flow_through_the_coupling() {
        let mut s = presets()[0].state.clone();
        s.coupling.insert("loudToFlow".into(), 1.0);
        let quiet = frame_params(&s, &AudioFeatures::default(), 0.0);
        let loud = frame_params(&s, &AudioFeatures { loudness: 1.0, ..Default::default() }, 0.0);
        assert!(loud.sim.flow.advect_amount > quiet.sim.flow.advect_amount);
    }
}
