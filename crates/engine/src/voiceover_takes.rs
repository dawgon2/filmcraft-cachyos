//! Voice-over takes (fork AUDIO-PLAN A2 item 2: per-take review).
//!
//! Every recording stays in the project. [`crate::voiceover`] tags each one with its *take group*
//! (sequence, record track and record point, in the item's free-form metadata, so projects need no
//! new fields) and its take number; recording again at the same point adds a take. The review
//! lists a clip's takes and puts the chosen one in its place, keeping the clip's effects, Essential
//! Sound settings and clip gain, so a clean-up applies to whichever take is used.
//!
//! | command | does |
//! |---|---|
//! | `audio.voiceover.takes` | the takes of a voice-over clip: item, name, take number, duration, which one is in use |
//! | `audio.voiceover.pickTake` | put another take of the same group at the record point (one undo step) |

use serde_json::{Value, json};

use filmcraft_project::{ClipId, ItemId, Project, TrackId, TrackKind};
use filmcraft_time::{Tick, TimeRange};

use crate::commands::{CommandSpec, bad, has_seq, u64_p};
use crate::{EngineError, Result, Session};

/// Metadata key of a recording's take group (`<sequence>/<track>/<record point ticks>`).
pub const TAKE_GROUP: &str = "Voice-over Take Group";
/// Metadata key of a recording's take number (1, 2, …).
pub const TAKE: &str = "Take";

pub fn group_key(seq: ItemId, track: TrackId, at: Tick) -> String {
    format!("{}/{}/{}", seq.0, track.0, at.0)
}

fn parse_group(g: &str) -> Option<(ItemId, TrackId, Tick)> {
    let mut it = g.split('/');
    let seq = it.next()?.parse().ok()?;
    let track = it.next()?.parse().ok()?;
    // free-form item metadata: a negative record point is damage, not a place on the timeline
    let at: i64 = it.next()?.parse().ok().filter(|t| *t >= 0)?;
    it.next().is_none().then_some((ItemId(seq), TrackId(track), Tick(at)))
}

fn take_no(pr: &Project, id: ItemId) -> u32 {
    pr.item(id).and_then(|i| i.metadata.get(TAKE)).and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// The project items of a take group, by take number.
fn takes_of(pr: &Project, group: &str) -> Vec<ItemId> {
    let mut v: Vec<ItemId> = pr.items.values().filter(|i| i.metadata.get(TAKE_GROUP).is_some_and(|g| g == group)).map(|i| i.id).collect();
    v.sort_by_key(|id| (take_no(pr, *id), id.0));
    v
}

/// The next take number of `group`.
pub fn next_take(pr: &Project, group: &str) -> u32 {
    takes_of(pr, group).iter().map(|id| take_no(pr, *id)).max().unwrap_or(0).saturating_add(1)
}

/// The clip a command acts on (`clip`, else the one selected audio clip) and its take group.
fn clip_and_group(s: &Session, p: &Value, cmd: &str) -> Result<(ClipId, ItemId, String)> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let clip = match u64_p(p, "clip") {
        Some(c) => ClipId(c),
        None => {
            let sel: Vec<ClipId> = s.state.selection.iter().copied().filter(|c| seq.audio_tracks.iter().any(|t| t.items.iter().any(|i| i.id == *c))).collect();
            match sel.as_slice() {
                [c] => *c,
                _ => return Err(bad(cmd, "select one voice-over clip (or pass `clip`)")),
            }
        }
    };
    let it = seq.audio_tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == clip).ok_or_else(|| bad(cmd, "no such audio clip"))?;
    let group = s.project.item(it.item).and_then(|i| i.metadata.get(TAKE_GROUP)).cloned().ok_or_else(|| bad(cmd, "this clip isn't a voice-over recording"))?;
    Ok((clip, it.item, group))
}

fn duration_of(s: &Session, id: ItemId) -> Option<Tick> {
    s.project.item(id).and_then(|i| i.as_media()).map(|m| m.info.duration)
}

/// One take of a clip's group (for the Essential Sound panel's Takes list).
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    pub item: ItemId,
    pub take: u32,
    pub duration: Option<Tick>,
    pub current: bool,
}

/// The takes of `clip` in the active sequence, by take number; empty when it isn't a recording.
pub fn list(s: &Session, clip: ClipId) -> Vec<Take> {
    let Ok((_, current, group)) = clip_and_group(s, &json!({"clip": clip.0}), "audio.voiceover.takes") else { return Vec::new() };
    takes_of(&s.project, &group)
        .into_iter()
        .map(|id| Take { item: id, take: take_no(&s.project, id), duration: duration_of(s, id), current: id == current })
        .collect()
}

fn takes(s: &mut Session, p: &Value) -> Result<Value> {
    let (clip, current, group) = clip_and_group(s, p, "audio.voiceover.takes")?;
    let list: Vec<Value> = takes_of(&s.project, &group)
        .into_iter()
        .map(|id| {
            json!({
                "item": id.0,
                "name": s.project.item(id).map(|i| i.name.clone()).unwrap_or_default(),
                "take": take_no(&s.project, id),
                "duration": duration_of(s, id).map(|d| d.0),
                "current": id == current,
            })
        })
        .collect();
    Ok(json!({"clip": clip.0, "group": group, "takes": list}))
}

fn pick_take(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "audio.voiceover.pickTake";
    let (clip, current, group) = clip_and_group(s, p, cmd)?;
    let take = ItemId(u64_p(p, "item").ok_or_else(|| bad(cmd, "need `item` (a take from audio.voiceover.takes)"))?);
    if !takes_of(&s.project, &group).contains(&take) {
        return Err(bad(cmd, "that item isn't a take of this clip"));
    }
    if take == current {
        return Ok(json!({"clip": clip.0, "item": take.0, "changed": false}));
    }
    let (seq_id, _, at) = parse_group(&group).ok_or_else(|| bad(cmd, "the clip's take group is damaged"))?;
    if s.state.active_sequence != Some(seq_id) {
        return Err(bad(cmd, "the take belongs to another sequence"));
    }
    let dur = duration_of(s, take).filter(|d| d.0 > 0).ok_or_else(|| bad(cmd, "that take has no audio"))?;
    let name = s.project.item(take).map(|i| i.name.clone()).unwrap_or_default();
    let label = format!("Use Take {}", take_no(&s.project, take));
    let new = s.edit(&label, |pr, st| {
        let rate = pr.sequence(seq_id).map(|q| q.settings.frame_rate).ok_or(EngineError::NoSequence)?;
        let mut it =
            pr.make_track_item(take, TrackKind::Audio, at, TimeRange::new(Tick::ZERO, dur), rate).ok_or_else(|| bad(cmd, "that take can't be placed"))?;
        it.duration = dur;
        let mut next = pr.next_id;
        let seq = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        let (track, old) = seq.find_item(clip).map(|(t, i)| (t, i.clone())).ok_or_else(|| bad(cmd, "no such audio clip"))?;
        // the clip's processing goes with it: effects (Essential Sound's among them), settings, gain
        it.effects = old.effects;
        it.essential = old.essential;
        it.gain_db = old.gain_db;
        it.label = old.label;
        let id = it.id;
        filmcraft_edit::delete_items(seq, &[clip]);
        let durations = |_: ItemId| -> Option<Tick> { None };
        let mut ctx =
            filmcraft_edit::EditCtx { next_id: &mut next, media_duration: &durations, media_start: &|_| Tick::ZERO, min_duration: rate.frame_duration() };
        filmcraft_edit::overwrite(seq, vec![(track, it)], &mut ctx)?;
        seq.check().map_err(EngineError::Other)?;
        pr.next_id = next;
        st.selection = vec![id];
        Ok(id)
    })?;
    Ok(json!({"clip": new.0, "item": take.0, "name": name, "changed": true}))
}

fn spec(id: &'static str, label: &'static str, params: &'static str, run: fn(&mut Session, &Value) -> Result<Value>, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: has_seq, run, journal }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        spec("audio.voiceover.takes", "Voice-over Takes", r#"{"clip":id?}"#, takes, false),
        spec("audio.voiceover.pickTake", "Use Take", r#"{"clip":id?,"item":id}"#, pick_take, true),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_damaged_record_point_is_refused() {
        assert!(super::parse_group("1/2/-5").is_none());
        assert!(super::parse_group("1/2/99999999999999999999").is_none());
        assert_eq!(super::parse_group("1/2/7").map(|g| g.2.0), Some(7));
    }
}
