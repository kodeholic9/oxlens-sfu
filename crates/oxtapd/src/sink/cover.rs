// author: kodeholic (powered by Claude)

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackCoverage {
    pub track_id: String,
    pub user_id: String,
    pub kind: String,
    pub listed_ms: u64,
    pub loaded_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub tracks: Vec<TrackCoverage>,
    pub not_recorded: Vec<String>,
}

pub(super) fn covered(entry: &Value) -> Option<(&str, &str, &str)> {
    let kind = entry["kind"].as_str().filter(|k| *k == "video" || *k == "audio")?;
    Some((entry["track_id"].as_str()?, entry["user_id"].as_str()?, kind))
}

#[derive(Clone, Debug, Default)]
pub(super) struct Cover {
    origin: Option<u64>,
    listed: BTreeMap<String, (String, String, u64)>,
    loaded: BTreeMap<String, u64>,
}

impl Cover {
    fn at(&mut self, log_time: u64) -> u64 {
        let origin = *self.origin.get_or_insert(log_time);
        log_time.saturating_sub(origin) / 1_000_000
    }

    pub(super) fn seen(&mut self, log_time: u64) {
        let _ = self.at(log_time);
    }

    pub(super) fn listed(&mut self, log_time: u64, data: &[u8]) {
        let ms = self.at(log_time);
        let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
        for t in v["tracks"].as_array().into_iter().flatten() {
            if let Some((tid, user, kind)) = covered(t) {
                self.listed.entry(tid.to_string()).or_insert_with(|| (user.to_string(), kind.to_string(), ms));
            }
        }
    }

    pub(super) fn loaded(&mut self, log_time: u64, track_id: &str) {
        let ms = self.at(log_time);
        self.loaded.entry(track_id.to_string()).or_insert(ms);
    }

    pub(super) fn summary(&self) -> Coverage {
        let tracks = self
            .listed
            .iter()
            .map(|(id, (user, kind, ms))| TrackCoverage { track_id: id.clone(), user_id: user.clone(), kind: kind.clone(), listed_ms: *ms, loaded_ms: self.loaded.get(id).copied() })
            .collect();
        let not_recorded = self.listed.keys().filter(|id| !self.loaded.contains_key(*id)).cloned().collect();
        Coverage { tracks, not_recorded }
    }
}
