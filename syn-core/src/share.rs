//! Point tokens: the `#s=` payload of the web app's long share links, which is
//! base64url of the point's JSON. There is no server here (PLAN.md decision
//! 9) — the token is just a way to carry a point between the two apps by
//! copy and paste.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::state::AppState;

/// Encodes a point as the token a web link would carry.
pub fn encode_token(state: &AppState) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_string(state).unwrap_or_default())
}

/// Decodes a token, or the whole link it was pasted from.
pub fn decode_token(token: &str) -> Option<AppState> {
    let raw = token.trim();
    // Accept a bare token, `#s=…`, or a full URL with `#s=` in it.
    let payload = match raw.split_once("#s=") {
        Some((_, rest)) => rest,
        None => raw.strip_prefix("s=").unwrap_or(raw),
    };
    let payload = payload.split(['&', ' ']).next()?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::presets;

    #[test]
    fn a_point_survives_the_round_trip() {
        for p in presets() {
            let token = encode_token(&p.state);
            let back = decode_token(&token).expect("decodes");
            assert_eq!(back, p.state, "{}", p.name);
        }
    }

    #[test]
    fn a_whole_link_is_accepted_too() {
        let state = &presets()[3].state;
        let token = encode_token(state);
        let link = format!("https://dmitryweiner.github.io/synesthesia/#s={token}");
        assert_eq!(decode_token(&link).as_ref(), Some(state));
        assert_eq!(decode_token(&format!("  #s={token}  ")).as_ref(), Some(state));
    }

    #[test]
    fn nonsense_is_rejected_rather_than_guessed() {
        assert!(decode_token("").is_none());
        assert!(decode_token("not a token").is_none());
        assert!(decode_token("###").is_none());
    }
}
