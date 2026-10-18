//! Output device order (no Windows dependencies — checked by tests). The Windows side is in
//! output.rs.

use crate::config::OutDev;

/// An output device as Windows sees it now.
#[derive(Clone, Debug)]
pub struct OutEp {
    pub id: String,
    pub name: String,
    /// DEVICE_STATE_*: 1 — active, 2 — disabled, 8 — not present (empty jack)
    pub state: u32,
}

pub const DEV_ACTIVE: u32 = 0x1;
pub const DEV_DISABLED: u32 = 0x2;
pub const DEV_UNPLUGGED: u32 = 0x8;

/// Disconnected devices beyond this are forgotten from the bottom of the list.
pub const MAX_OUT_DEVS: usize = 40;

/// The saved order + devices not in it yet (at the end). An empty order (first run): the headset
/// (its "Game"), then `prefer` (the device switched from before, and the default), then the rest,
/// disabled ones lower, virtual cables at the very end.
pub fn merge_out_order(saved: &[OutDev], eps: &[OutEp], prefer: &[String]) -> (Vec<OutDev>, bool) {
    let mut out: Vec<OutDev> = Vec::with_capacity(saved.len() + eps.len());
    let mut changed = false;
    for d in saved {
        if d.id.is_empty() || out.iter().any(|x| x.id == d.id) {
            changed = true;
            continue;
        }
        out.push(d.clone());
    }
    let mut add = |out: &mut Vec<OutDev>, e: &OutEp| {
        if let Some(x) = out.iter_mut().find(|x| x.id == e.id) {
            if !e.name.is_empty() && x.name != e.name {
                x.name = e.name.clone();
                changed = true;
            }
            return;
        }
        out.push(OutDev { id: e.id.clone(), name: e.name.clone() });
        changed = true;
    };
    if saved.is_empty() {
        // the headset first: its "Game" first (models with separate Game and Chat), "Chat" last of them
        for rank in [3, 1, 0] {
            for e in eps.iter().filter(|e| is_headset_name(&e.name) && headset_rank(&e.name) == rank) {
                add(&mut out, e);
            }
        }
        for id in prefer.iter().filter(|id| !id.is_empty()) {
            for e in eps.iter().filter(|e| e.id == *id && !is_virtual_name(&e.name)) {
                add(&mut out, e);
            }
        }
        for st in [DEV_ACTIVE, DEV_UNPLUGGED, DEV_DISABLED] {
            for e in eps.iter().filter(|e| e.state == st && !is_virtual_name(&e.name)) {
                add(&mut out, e);
            }
        }
    }
    for e in eps {
        add(&mut out, e);
    }
    // too many remembered — drop the lowest of those Windows no longer sees
    if out.len() > MAX_OUT_DEVS {
        let mut i = out.len();
        while i > 0 && out.len() > MAX_OUT_DEVS {
            i -= 1;
            if !eps.iter().any(|e| e.id == out[i].id) {
                out.remove(i);
                changed = true;
            }
        }
    }
    (out, changed)
}

/// Virtual audio cables (VB-CABLE, Hi-Fi Cable, VAC, Voicemeeter and the like).
pub fn is_virtual_name(n: &str) -> bool {
    let l = n.to_lowercase();
    ["cable", "vb-audio", "virtual", "voicemeeter", "vaio", "line 1", "line 2", "line 3"].iter().any(|k| l.contains(k))
}

/// A device of the headset the app works with.
pub fn is_headset_name(name: &str) -> bool {
    let n = name.to_lowercase();
    !is_virtual_name(name) && (n.contains("arctis") || n.contains("steelseries"))
}

/// For headsets with separate Game and Chat devices the sound goes to "Game".
pub fn headset_rank(name: &str) -> i32 {
    let n = name.to_lowercase();
    if n.contains("game") {
        3
    } else if n.contains("chat") {
        0
    } else {
        1
    }
}

/// The highest device in the list that sound can go to now.
pub fn out_top(list: &[super::OutDevJson]) -> String {
    list.iter().find(|d| d.state == "on").map(|d| d.id.clone()).unwrap_or_default()
}

/// A new order from the window (dragging). Unknown ids are skipped, devices missing from the new
/// order stay at the end in the same order.
pub fn reorder_out(order: &[OutDev], ids: &[String]) -> Vec<OutDev> {
    let mut out: Vec<OutDev> = Vec::with_capacity(order.len());
    for id in ids {
        if let Some(d) = order.iter().find(|d| d.id == *id) {
            if !out.iter().any(|x| x.id == *id) {
                out.push(d.clone());
            }
        }
    }
    for d in order {
        if !out.iter().any(|x| x.id == d.id) {
            out.push(d.clone());
        }
    }
    out
}

/// Moves a device by delta positions (-1 — higher).
pub fn move_out(order: &[OutDev], id: &str, delta: i32) -> Vec<OutDev> {
    let mut out = order.to_vec();
    if let Some(i) = out.iter().position(|d| d.id == id) {
        let j = i as i32 + delta;
        if j >= 0 && (j as usize) < out.len() {
            out.swap(i, j as usize);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::OutDevJson;

    fn ids(l: &[OutDev]) -> String {
        l.iter().map(|d| d.id.as_str()).collect::<Vec<_>>().join(",")
    }

    fn ep(id: &str, name: &str, state: u32) -> OutEp {
        OutEp { id: id.into(), name: name.into(), state }
    }

    fn od(id: &str, name: &str) -> OutDev {
        OutDev { id: id.into(), name: name.into() }
    }

    /// First run: the headset (its "Game") first, then the device switched from and the default,
    /// then the rest, disabled lower, virtual cables at the end.
    #[test]
    fn first_run() {
        let eps = [
            ep("cab", "CABLE Input (VB-Audio Virtual Cable)", DEV_ACTIVE),
            ep("mon", "DELL (NVIDIA High Definition Audio)", DEV_ACTIVE),
            ep("chat", "Headset (Arctis Nova Pro Chat)", DEV_ACTIVE),
            ep("off", "Speakers (Realtek)", DEV_DISABLED),
            ep("fr", "Speakers (Focusrite USB Audio)", DEV_ACTIVE),
            ep("game", "Headset (Arctis Nova Pro Game)", DEV_ACTIVE),
        ];
        let (got, changed) = merge_out_order(&[], &eps, &["fr".into(), "game".into()]);
        assert!(changed);
        assert_eq!(ids(&got), "game,chat,fr,mon,off,cab");
    }

    /// The saved order stays; a new device goes to the end; names update; duplicates vanish.
    #[test]
    fn keep() {
        let saved = [od("fr", "Focusrite"), od("gone", "Old headset"), od("hs", "Arctis 7+"), od("fr", "dup")];
        let eps = [ep("hs", "Headphones (Arctis 7+)", DEV_ACTIVE), ep("fr", "Focusrite", DEV_ACTIVE), ep("new", "USB DAC", DEV_ACTIVE)];
        let (got, changed) = merge_out_order(&saved, &eps, &["hs".into()]);
        assert!(changed);
        assert_eq!(ids(&got), "fr,gone,hs,new");
        assert_eq!(got[2].name, "Headphones (Arctis 7+)");
        assert_eq!(got[1].name, "Old headset");
        let (again, changed) = merge_out_order(&got, &eps, &["hs".into()]);
        assert!(!changed && ids(&again) == ids(&got), "no changes should keep it as is");
    }

    /// Disconnected devices are not remembered without limit.
    #[test]
    fn limit() {
        let saved: Vec<OutDev> = (0..MAX_OUT_DEVS + 5).map(|i| od(&format!("{}{}", (b'A' + (i % 26) as u8) as char, (b'a' + (i / 26) as u8) as char), "x")).collect();
        let eps = [ep(&saved[saved.len() - 1].id, "x", DEV_ACTIVE)];
        let (got, _) = merge_out_order(&saved, &eps, &[]);
        assert_eq!(got.len(), MAX_OUT_DEVS);
        assert_eq!(got.last().unwrap().id, saved.last().unwrap().id);
    }

    #[test]
    fn top_and_move() {
        let j = |id: &str, st: &str| OutDevJson { id: id.into(), state: st.into(), ..Default::default() };
        assert_eq!(out_top(&[j("a", "hsoff"), j("b", "disabled"), j("c", "on"), j("d", "on")]), "c");
        let o = [od("a", ""), od("b", ""), od("c", "")];
        assert_eq!(ids(&move_out(&o, "c", -1)), "a,c,b");
        assert_eq!(ids(&move_out(&o, "a", -1)), "a,b,c");
        assert_eq!(ids(&reorder_out(&o, &["c".into(), "x".into(), "a".into(), "c".into()])), "c,a,b");
    }
}
