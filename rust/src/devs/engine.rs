//! The device description engine — a port of part of Linux-Arctis-Manager (daemon/device-config:
//! codec, api_executor, sync_dispatcher, sync_reader, transform_eval, builtins, biquad). The
//! descriptions come from their YAML (turned into web/devices.json by tools/gen_devices.py).
//! No Windows dependencies here — checked by tests on any OS.

use std::{collections::BTreeMap, collections::HashMap, time::Duration};

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::builtins;

// ---------- the description ----------

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevField {
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub constant: Option<Value>,
    pub range: Vec<Value>,
    pub values: Vec<Value>,
    pub values_mapping: BTreeMap<String, String>,
    pub repeat: i64,
    pub size: i64,
    /// a reference {struct: name}
    #[serde(rename = "struct")]
    pub strukt: String,
}

/// Flat (the same fields both ways) or outgoing / incoming.
#[derive(Clone, Debug, Default)]
pub struct DevStruct {
    pub out: Vec<DevField>,
    pub inp: Vec<DevField>,
}

impl<'de> Deserialize<'de> for DevStruct {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if v.is_array() {
            let l: Vec<DevField> = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
            return Ok(DevStruct { out: l.clone(), inp: l });
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Bi {
            outgoing: Vec<DevField>,
            incoming: Vec<DevField>,
        }
        let b: Bi = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
        Ok(DevStruct { out: b.outgoing, inp: b.incoming })
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevOp {
    pub transport: String,
    pub chunk_size: usize,
    #[serde(rename = "payload_transform")]
    pub transform: String,
    #[serde(rename = "payload_transform_args")]
    pub args: BTreeMap<String, Value>,
    pub sleep_ms: u64,
}

/// One message is one step; or a sequence of steps.
#[derive(Clone, Debug, Default)]
pub struct DevWrite {
    pub steps: Vec<DevOp>,
}

impl<'de> Deserialize<'de> for DevWrite {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if let Some(steps) = v.get("steps").filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) {
            let steps: Vec<DevOp> = serde_json::from_value(steps.clone()).map_err(serde::de::Error::custom)?;
            return Ok(DevWrite { steps });
        }
        let op: DevOp = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
        Ok(DevWrite { steps: vec![op] })
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevApi {
    pub read: Option<DevOp>,
    pub write: Option<DevWrite>,
    pub action: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevTransform {
    #[serde(rename = "type")]
    pub typ: String,
    pub default: Option<Value>,
    pub values: BTreeMap<String, Value>,
    pub scale: f64,
    pub offset: f64,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevEventField {
    pub name: String,
    pub byte: usize,
    pub transform: String,
    pub display_type: String,
    pub role: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevSideEffect {
    pub call: String,
    pub arg_byte: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevEvent {
    pub emit: String,
    pub fields: Vec<DevEventField>,
    pub side_effects: Vec<DevSideEffect>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevReadMap {
    pub emit: String,
    pub field: String,
    pub fields: Vec<String>,
    pub transform: String,
    pub display_type: String,
    pub role: String,
    pub roles: Vec<Value>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevReadEntry {
    #[serde(rename = "struct")]
    pub strukt: String,
    pub maps: Vec<DevReadMap>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevCall {
    pub call: String,
    pub args: Option<Value>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevIface {
    pub interface: i32,
    pub usage_page: i32,
    pub usage: i32,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Variant {
    pub name: String,
    pub product_id: i32,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevHid {
    pub usage_page: i32,
    pub usage: i32,
    #[serde(rename = "command_interface")]
    pub command: Option<DevIface>,
    #[serde(rename = "sync_interface")]
    pub sync: Option<DevIface>,
    #[serde(rename = "unnumbered_reports")]
    pub unnumbered: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevDevice {
    pub name: String,
    pub vendor_id: i32,
    pub variants: Vec<Variant>,
    pub hid: DevHid,
    pub capabilities: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct DevConfig {
    pub file: String,
    pub constants: HashMap<String, Value>,
    pub structs: HashMap<String, DevStruct>,
    pub apis: HashMap<String, DevApi>,
    #[serde(rename = "transforms")]
    pub trans: HashMap<String, DevTransform>,
    #[serde(rename = "sync_events")]
    pub events: HashMap<String, DevEvent>,
    pub sync_read: Vec<DevReadEntry>,
    pub lifecycle: HashMap<String, Vec<DevCall>>,
    pub device: DevDevice,
}

pub fn parse_devices(b: &str) -> Result<Vec<DevConfig>, serde_json::Error> {
    let mut l: Vec<DevConfig> = serde_json::from_str(b)?;
    for c in l.iter_mut() {
        quirks(c);
    }
    Ok(l)
}

/// Corrections to the Linux-Arctis-Manager descriptions, checked against HeadsetControl and the Linux
/// kernel driver (hid-steelseries-arctis); web/devices.json stays as generated.
fn quirks(c: &mut DevConfig) {
    // the ChatMix values to 0..100 % in every map that reads them
    fn chatmix_transform(c: &mut DevConfig, name: &str, t: DevTransform) {
        c.trans.insert(name.into(), t);
        for e in c.sync_read.iter_mut() {
            for m in e.maps.iter_mut().filter(|m| m.emit == "chatmix_changed") {
                m.transform = name.into();
            }
        }
    }
    match c.file.as_str() {
        "arctis_7.yaml" => {
            // commands and answers go through interface 5 (interface 3 does not answer)
            for i in [&mut c.device.hid.command, &mut c.device.hid.sync] {
                i.get_or_insert_with(Default::default).interface = 5;
            }
            // each channel: 0 — full volume, else 191..255 (quietest..loudest)
            let t = DevTransform { typ: "linear".into(), scale: 100.0 / 64.0, offset: -191.0 * 100.0 / 64.0, values: [("0".to_string(), Value::from(100))].into(), ..Default::default() };
            chatmix_transform(c, "arctis_7_chatmix_to_percent", t);
        }
        "arctis_9.yaml" => {
            // each channel 0..19
            let t = DevTransform { typ: "linear".into(), scale: 100.0 / 19.0, ..Default::default() };
            chatmix_transform(c, "arctis_9_chatmix_to_percent", t);
            // the battery, not mapped in the description (HeadsetControl): the level 0x64..0x9A, 1 after it — charging
            let lvl = DevTransform { typ: "linear".into(), scale: 100.0 / 54.0, offset: -100.0 * 100.0 / 54.0, ..Default::default() };
            c.trans.insert("arctis_9_battery_to_percent".into(), lvl);
            let chg = DevTransform { typ: "case_int_to_str".into(), values: [("1".to_string(), Value::from("CHARGING"))].into(), default: Some(Value::from("NOT_CHARGING")), ..Default::default() };
            c.trans.insert("arctis_9_charging_to_label".into(), chg);
            if let Some(e) = c.sync_read.iter_mut().find(|e| e.strukt == "info_table") {
                e.maps.push(DevReadMap { emit: "battery_changed".into(), field: "bat_voltage_lo".into(), transform: "arctis_9_battery_to_percent".into(), role: "battery_headset".into(), ..Default::default() });
                e.maps.push(DevReadMap { emit: "charging_changed".into(), field: "bat_status".into(), transform: "arctis_9_charging_to_label".into(), ..Default::default() });
            }
        }
        // a wired USB headset, though its description lists it as wireless (the window would show 2.4 GHz)
        "arctis_5.yaml" => c.device.capabilities.retain(|x| x != "wireless"),
        _ => {}
    }
}

/// Only the model codes from the descriptions — to recognise a headset without keeping all the full
/// descriptions in memory (they are parsed when a matching one is found).
pub fn device_pids(b: &str) -> Result<std::collections::HashSet<i32>, serde_json::Error> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct V {
        product_id: i32,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct D {
        variants: Vec<V>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct C {
        device: D,
    }
    let l: Vec<C> = serde_json::from_str(b)?;
    Ok(l.iter().flat_map(|c| c.device.variants.iter().map(|v| v.product_id)).collect())
}

impl DevConfig {
    /// The name of a model by PID.
    pub fn variant_name(&self, pid: i32) -> String {
        self.device.variants.iter().find(|v| v.product_id == pid && !v.name.is_empty()).map(|v| v.name.clone()).unwrap_or_else(|| self.device.name.clone())
    }

    pub fn has_pid(&self, pid: i32) -> bool {
        self.device.variants.iter().any(|v| v.product_id == pid)
    }
}

// ---------- field values ----------

#[derive(Clone, Debug, PartialEq)]
pub enum FVal {
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    Arr(Vec<FVal>),
}

impl FVal {
    pub fn num(&self) -> f64 {
        if let FVal::Num(v) = self { *v } else { 0.0 }
    }
}

impl std::fmt::Display for FVal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FVal::Num(v) => write!(f, "{v}"),
            FVal::Str(s) => write!(f, "{s}"),
            FVal::Bytes(b) => write!(f, "{}", b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")),
            FVal::Arr(a) => write!(f, "[{}]", a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")),
        }
    }
}

pub fn any_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok().or_else(|| {
            let l = s.to_lowercase();
            l.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).map(|u| u as f64)
        }),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

impl DevConfig {
    pub fn resolve<'a>(&'a self, v: &'a Value) -> &'a Value {
        if let Value::String(s) = v {
            if let Some(name) = s.strip_prefix('$') {
                if let Some(r) = self.constants.get(name) {
                    return r;
                }
            }
        }
        v
    }

    fn resolve_num(&self, v: &Value) -> Option<f64> {
        any_num(self.resolve(v))
    }
}

// ---------- codec ----------

fn type_size(t: &str, size: i64) -> usize {
    match t {
        "uint8" | "int8" => 1,
        "uint16" | "int16" => 2,
        "uint32" | "int32" | "float32" => 4,
        "bytearray" => size.max(0) as usize,
        _ => 0,
    }
}

fn write_num(buf: &mut Vec<u8>, t: &str, v: f64) {
    let r = v.round();
    match t {
        "uint8" => buf.push(r as i64 as u64 as u8),
        "int8" => buf.push(r as i64 as i8 as u8),
        "uint16" => buf.extend((r as i64 as u16).to_be_bytes()),
        "int16" => buf.extend((r as i64 as i16 as u16).to_be_bytes()),
        "uint32" => buf.extend((r as i64 as u32).to_be_bytes()),
        "int32" => buf.extend((r as i64 as i32 as u32).to_be_bytes()),
        "float32" => buf.extend((v as f32).to_bits().to_be_bytes()),
        _ => {}
    }
}

fn write_val(buf: &mut Vec<u8>, t: &str, v: &FVal) {
    match v {
        FVal::Str(s) => buf.extend(s.as_bytes()),
        FVal::Bytes(b) => buf.extend(b),
        FVal::Arr(a) => a.iter().for_each(|e| write_val(buf, t, e)),
        FVal::Num(n) => write_num(buf, t, *n),
    }
}

fn read_num(b: &[u8], t: &str) -> f64 {
    let be16 = || u16::from_be_bytes([b[0], b[1]]);
    let be32 = || u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    match t {
        "uint8" => b[0] as f64,
        "int8" => b[0] as i8 as f64,
        "uint16" => be16() as f64,
        "int16" => be16() as i16 as f64,
        "uint32" => be32() as f64,
        "int32" => be32() as i32 as f64,
        "float32" => f32::from_bits(be32()) as f64,
        _ => 0.0,
    }
}

impl DevConfig {
    fn encode_fields(&self, fields: &[DevField], sname: &str, vals: &HashMap<String, FVal>, buf: &mut Vec<u8>) -> Result<(), String> {
        for f in fields {
            if !f.strukt.is_empty() {
                let st = self.structs.get(&f.strukt).ok_or_else(|| format!("unknown struct {}", f.strukt))?;
                self.encode_fields(&st.out, &f.strukt, vals, buf)?;
                continue;
            }
            let count = f.repeat.max(1) as usize;
            if let Some(c) = &f.constant {
                let n = self.resolve_num(c).ok_or_else(|| format!("field {}: non-numeric constant", f.name))?;
                for _ in 0..count {
                    write_num(buf, &f.typ, n);
                }
                continue;
            }
            let v = vals.get(&f.name).ok_or_else(|| format!("{sname}: no value for field {}", f.name))?;
            if f.typ == "varstring" {
                let s = if let FVal::Str(s) = v { s.as_bytes() } else { &[][..] };
                if f.size > 0 {
                    let n = s.len().min(f.size as usize);
                    buf.extend(&s[..n]);
                    buf.extend(std::iter::repeat_n(0u8, f.size as usize - n));
                } else {
                    buf.extend(s);
                }
                continue;
            }
            if count == 1 || matches!(v, FVal::Arr(_)) {
                write_val(buf, &f.typ, v);
            } else {
                for _ in 0..count {
                    write_val(buf, &f.typ, v);
                }
            }
        }
        Ok(())
    }

    pub fn serialize(&self, sname: &str, vals: &HashMap<String, FVal>) -> Result<Vec<u8>, String> {
        let st = self.structs.get(sname).ok_or_else(|| format!("unknown struct {sname}"))?;
        let mut buf = Vec::new();
        self.encode_fields(&st.out, sname, vals, &mut buf)?;
        Ok(buf)
    }

    fn decode_fields(&self, fields: &[DevField], b: &[u8], cur: &mut usize, out: &mut HashMap<String, FVal>) -> Result<(), String> {
        for f in fields {
            if !f.strukt.is_empty() {
                let st = self.structs.get(&f.strukt).ok_or_else(|| format!("unknown struct {}", f.strukt))?;
                self.decode_fields(&st.inp, b, cur, out)?;
                continue;
            }
            if f.typ == "varstring" {
                let mut raw = &b[(*cur).min(b.len())..];
                *cur = b.len();
                if let Some(i) = raw.iter().position(|&x| x == 0) {
                    raw = &raw[..i];
                }
                out.insert(f.name.clone(), FVal::Str(String::from_utf8_lossy(raw).into_owned()));
                continue;
            }
            let n = type_size(&f.typ, f.size);
            let count = f.repeat.max(1) as usize;
            let read = |cur: &mut usize| -> Result<FVal, String> {
                if *cur + n > b.len() || n == 0 {
                    return Err(format!("response too short: field {}", f.name));
                }
                let s = &b[*cur..*cur + n];
                *cur += n;
                Ok(if f.typ == "bytearray" { FVal::Bytes(s.to_vec()) } else { FVal::Num(read_num(s, &f.typ)) })
            };
            if count == 1 {
                let v = read(cur)?;
                if let Some(c) = &f.constant {
                    if let Some(want) = self.resolve_num(c) {
                        if want != v.num() {
                            return Err(format!("field {}: expected {:#x}, got {:#x}", f.name, want as i64, v.num() as i64));
                        }
                    }
                    continue;
                }
                out.insert(f.name.clone(), v);
            } else {
                let mut arr = Vec::with_capacity(count);
                for _ in 0..count {
                    arr.push(read(cur)?);
                }
                if f.constant.is_none() {
                    out.insert(f.name.clone(), FVal::Arr(arr));
                }
            }
        }
        Ok(())
    }

    pub fn deserialize(&self, sname: &str, b: &[u8]) -> Result<HashMap<String, FVal>, String> {
        let st = self.structs.get(sname).ok_or_else(|| format!("unknown struct {sname}"))?;
        let mut out = HashMap::new();
        let mut cur = 0;
        self.decode_fields(&st.inp, b, &mut cur, &mut out)?;
        Ok(out)
    }

    /// All write fields (with references expanded).
    pub fn out_fields(&self, sname: &str) -> Vec<DevField> {
        fn walk(c: &DevConfig, fs: &[DevField], out: &mut Vec<DevField>) {
            for f in fs {
                if !f.strukt.is_empty() {
                    if let Some(s) = c.structs.get(&f.strukt) {
                        walk(c, &s.out, out);
                    }
                    continue;
                }
                out.push(f.clone());
            }
        }
        let mut out = Vec::new();
        if let Some(st) = self.structs.get(sname) {
            walk(self, &st.out, &mut out);
        }
        out
    }

    // ---------- transforms ----------

    pub fn transform(&self, name: &str, v: FVal) -> FVal {
        let (Some(t), FVal::Num(n)) = (self.trans.get(name), &v) else { return v };
        let key = (*n as i64).to_string();
        match t.typ.as_str() {
            "case_int_to_int" => {
                if let Some(r) = t.values.get(&key).and_then(any_num) {
                    return FVal::Num(r);
                }
                t.default.as_ref().and_then(any_num).map(FVal::Num).unwrap_or(v)
            }
            "case_int_to_str" => {
                let s = |x: &Value| match x {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if let Some(r) = t.values.get(&key) {
                    return FVal::Str(s(r));
                }
                FVal::Str(t.default.as_ref().map(s).unwrap_or_else(|| "UNKNOWN".into()))
            }
            // listed values are taken as they are, the rest scaled
            "linear" => t.values.get(&key).and_then(any_num).map_or(FVal::Num(n * t.scale + t.offset), FVal::Num),
            _ => v,
        }
    }
}

// ---------- events ----------

#[derive(Debug, Default)]
pub struct DevEmit {
    pub signal: String,
    pub fields: HashMap<String, FVal>,
    /// field → role (battery_headset, chatmix_game, …)
    pub roles: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct DevDispatch {
    pub emit: Option<DevEmit>,
    pub side_effects: Vec<String>,
}

impl DevConfig {
    /// An asynchronous report from the headset (byte 1 — the command code).
    pub fn dispatch(&self, r: &[u8]) -> Option<DevDispatch> {
        if r.len() < 2 {
            return None;
        }
        let ev = self.events.get(&r[1].to_string())?;
        let mut d = DevDispatch::default();
        if !ev.emit.is_empty() {
            let mut e = DevEmit { signal: ev.emit.clone(), ..Default::default() };
            for f in &ev.fields {
                if f.byte >= r.len() {
                    return None;
                }
                let mut v = FVal::Num(r[f.byte] as f64);
                if !f.transform.is_empty() {
                    v = self.transform(&f.transform, v);
                }
                e.fields.insert(f.name.clone(), v);
                if !f.role.is_empty() {
                    e.roles.insert(f.name.clone(), f.role.clone());
                }
            }
            d.emit = Some(e);
        }
        d.side_effects = ev.side_effects.iter().map(|s| s.call.clone()).collect();
        Some(d)
    }

    /// An answer to polling (sync_read) → events.
    pub fn map_read(&self, e: &DevReadEntry, fields: &HashMap<String, FVal>) -> Vec<DevEmit> {
        let mut out = Vec::new();
        for m in &e.maps {
            let mut em = DevEmit { signal: m.emit.clone(), ..Default::default() };
            let names: Vec<String> = if m.field.is_empty() { m.fields.clone() } else { vec![m.field.clone()] };
            for (i, n) in names.iter().enumerate() {
                let Some(v) = fields.get(n) else { continue };
                let mut v = v.clone();
                if !m.transform.is_empty() {
                    v = self.transform(&m.transform, v);
                }
                em.fields.insert(n.clone(), v);
                if !m.field.is_empty() && !m.role.is_empty() {
                    em.roles.insert(n.clone(), m.role.clone());
                }
                if m.field.is_empty() {
                    if let Some(Value::String(r)) = m.roles.get(i) {
                        if !r.is_empty() {
                            em.roles.insert(n.clone(), r.clone());
                        }
                    }
                }
            }
            out.push(em);
        }
        out
    }
}

// ---------- write / read (API) ----------

#[derive(Debug, Clone, Default)]
pub struct DevAction {
    pub payload: Vec<u8>,
    pub feature: bool,
    pub sleep: Duration,
}

fn pad_to(mut b: Vec<u8>, n: usize) -> Vec<u8> {
    if b.len() < n {
        b.resize(n, 0);
    }
    b
}

impl DevConfig {
    pub fn prepare_write(&self, api: &str, vals: &HashMap<String, FVal>) -> Result<Vec<DevAction>, String> {
        let a = self.apis.get(api).and_then(|a| a.write.as_ref()).ok_or_else(|| format!("API {api} has no write"))?;
        let bytes = self.serialize(api, vals)?;
        let mut out = Vec::new();
        for st in &a.steps {
            if st.sleep_ms > 0 && st.transport.is_empty() {
                out.push(DevAction { sleep: Duration::from_millis(st.sleep_ms), ..Default::default() });
                continue;
            }
            let payloads = if st.transform.is_empty() {
                vec![bytes.clone()]
            } else {
                let f = builtins::get(&st.transform).ok_or_else(|| format!("unknown transform {}", st.transform))?;
                f(bytes.clone(), &st.args)
            };
            for p in payloads {
                out.push(DevAction { payload: pad_to(p, st.chunk_size), feature: st.transport == "HID_FEATURE", sleep: Duration::ZERO });
            }
        }
        Ok(out)
    }

    pub fn prepare_read(&self, api: &str) -> Result<(Vec<u8>, DevOp), String> {
        let op = self.apis.get(api).and_then(|a| a.read.clone()).ok_or_else(|| format!("API {api} has no read"))?;
        let b = self.serialize(api, &HashMap::new())?;
        Ok((pad_to(b, op.chunk_size), op))
    }
}

/// Lifecycle calls that just write one field: (api, field, value).
pub fn simple_call(name: &str) -> Option<(&'static str, &'static str, f64)> {
    Some(match name {
        "enable_sonar" => ("set_sonar_present", "is_present", 1.0),
        "disable_sonar" => ("set_sonar_present", "is_present", 0.0),
        "reset_eq_preset" => ("selected_eq_preset", "eq_preset", 0.0),
        "enable_chatmix" => ("software_chatmix_status", "status", 1.0),
        "disable_chatmix" => ("software_chatmix_status", "status", 0.0),
        _ => return None,
    })
}

// ---------- settings for the window ----------

use crate::devices::{DevEq, DevSetting, DevSettingOpt};

/// Service APIs not shown as plain settings.
const HIDDEN_APIS: [&str; 10] = [
    "save_to_flash",
    "av6x02_init",
    "set_sonar_present",
    "software_chatmix_status",
    "selected_eq_preset",
    "select_eq_preset",
    "stream_mix",
    "hp_speaker_mix",
    "current_volume",
    "reset",
];

fn is_eq_api(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("eq") || n == "equalizer"
}

fn fmt_num(v: f64) -> String {
    format!("{v}")
}

fn opt(v: f64, label: impl Into<String>) -> DevSettingOpt {
    DevSettingOpt { v, label: label.into() }
}

impl DevConfig {
    pub fn settings_schema(&self) -> Vec<DevSetting> {
        let mut out = Vec::new();
        let mut names: Vec<&String> = self.apis.keys().collect();
        names.sort();
        for n in names {
            let a = &self.apis[n];
            if a.write.is_none() || a.action || HIDDEN_APIS.contains(&n.as_str()) || is_eq_api(n) || !self.structs.contains_key(n) {
                continue;
            }
            for f in self.out_fields(n) {
                if f.constant.is_some() || ["varstring", "bytearray", "float32"].contains(&f.typ.as_str()) {
                    continue;
                }
                let mut s = DevSetting { api: n.clone(), field: f.name.clone(), label: setting_label(n, &f.name), ..Default::default() };
                if !f.values_mapping.is_empty() {
                    s.kind = "seg".into();
                    let mut keys: Vec<(f64, &String)> = f.values_mapping.iter().filter_map(|(k, v)| any_num(&Value::String(k.clone())).map(|x| (x, v))).collect();
                    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                    s.opts = keys.into_iter().map(|(k, v)| opt(k, value_label(v))).collect();
                } else if f.range.len() == 2 {
                    let lo = self.resolve_num(&f.range[0]).unwrap_or(0.0);
                    let hi = self.resolve_num(&f.range[1]).unwrap_or(0.0);
                    (s.min, s.max) = (lo, hi);
                    if hi - lo <= 1.0 {
                        s.kind = "toggle".into();
                    } else if hi - lo <= 6.0 {
                        s.kind = "seg".into();
                        let mut v = lo;
                        while v <= hi {
                            s.opts.push(opt(v, fmt_num(v)));
                            v += 1.0;
                        }
                    } else {
                        s.kind = "slider".into();
                    }
                } else {
                    continue;
                }
                let lname = format!("{n} {}", f.name).to_lowercase();
                let is_timer = lname.contains("timer") || lname.contains("off_time") || lname.contains("minutes") || n == "inactivity_timer";
                if is_timer && s.max >= 30.0 && f.name != "dim_timer" {
                    s.kind = "seg".into();
                    s.opts.clear();
                    for m in [0.0, 5.0, 10.0, 15.0, 30.0, 60.0] {
                        if m >= s.min && m <= s.max {
                            s.opts.push(opt(m, if m == 0.0 { "Ніколи".to_string() } else { format!("{} хв", m as i64) }));
                        }
                    }
                }
                let plain_seg = |s: &DevSetting, max: f64| s.kind == "seg" && s.min == 0.0 && s.max == max && f.values_mapping.is_empty();
                if lname.contains("led") && plain_seg(&s, 2.0) {
                    s.opts = vec![opt(0.0, "Вимк."), opt(1.0, "Постійно"), opt(2.0, "Пульсує")];
                }
                if lname.contains("sidetone") && plain_seg(&s, 3.0) {
                    s.opts = vec![opt(0.0, "Вимк."), opt(1.0, "Низький"), opt(2.0, "Середній"), opt(3.0, "Високий")];
                }
                if lname.contains("brightness") && plain_seg(&s, 3.0) {
                    s.opts = vec![opt(0.0, "Вимк."), opt(1.0, "Низька"), opt(2.0, "Середня"), opt(3.0, "Висока")];
                }
                if s.kind == "seg" && !s.opts.is_empty() {
                    (s.min, s.max) = (s.opts[0].v, s.opts[s.opts.len() - 1].v);
                }
                out.push(s);
            }
        }
        out
    }

    // ---------- equalizer ----------

    /// The preset selection API (selected_eq_preset / select_eq_preset).
    fn attach_presets(&self, e: &mut DevEq) {
        for name in ["selected_eq_preset", "select_eq_preset"] {
            if self.apis.get(name).is_none_or(|a| a.write.is_none()) {
                continue;
            }
            for f in self.out_fields(name) {
                if f.constant.is_some() || f.values_mapping.is_empty() {
                    continue;
                }
                e.preset_api = name.into();
                e.preset_field = f.name.clone();
                let mut keys: Vec<(f64, &String)> = f.values_mapping.iter().filter_map(|(k, v)| any_num(&Value::String(k.clone())).map(|x| (x, v))).collect();
                keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (k, raw) in keys {
                    if raw.eq_ignore_ascii_case("custom") {
                        e.custom_slot = k;
                        e.has_custom = true;
                        continue;
                    }
                    e.presets.push(opt(k, preset_label(raw)));
                }
                return;
            }
        }
    }

    /// Finds a hardware equalizer that can be driven as a set of bands.
    pub fn detect_eq(&self) -> Option<DevEq> {
        for name in ["eq", "custom_eq", "equalizer", "custom_eq_all"] {
            if self.apis.get(name).is_none_or(|a| a.write.is_none()) {
                continue;
            }
            let fs = self.out_fields(name);
            let gains: Vec<&DevField> = fs.iter().filter(|f| f.constant.is_none() && (f.name.starts_with("gain") || f.name.starts_with("band_") || f.name.starts_with("custom_eq"))).collect();
            if gains.len() < 3 {
                continue;
            }
            let mut e = DevEq { api: name.into(), kind: "graphic".into(), bands: gains.len(), min: -12.0, max: 12.0, ..Default::default() };
            for g in &gains {
                e.fields.push(g.name.clone());
                if g.range.len() == 2 {
                    let lo = self.resolve_num(&g.range[0]).unwrap_or(0.0);
                    let hi = self.resolve_num(&g.range[1]).unwrap_or(0.0);
                    (e.min, e.max) = if g.typ == "float32" { (lo.max(-12.0), hi.min(12.0)) } else { (lo, hi) };
                }
            }
            e.freqs = self.eq_freqs(name, &gains);
            self.attach_presets(&mut e);
            return Some(e);
        }
        // parametric (Nova 5, Nova 7 Gen2, Nova 3 Wireless, GameBuds): 10 "peak" bands on standard frequencies
        if let Some(w) = self.apis.get("parametric_eq").and_then(|a| a.write.as_ref()) {
            let fs = self.out_fields("parametric_eq");
            if fs.iter().any(|f| f.name == "band1_gain") && fs.iter().any(|f| f.name == "preset_type") {
                let mut e = DevEq { api: "parametric_eq".into(), kind: "parametric".into(), bands: 10, freqs: crate::devices::STD10.to_vec(), min: -10.0, max: 10.0, ..Default::default() };
                if w.steps.iter().any(|st| st.args.get("gain_flavour").and_then(|v| v.as_str()) == Some("signed_tenths_db")) {
                    (e.min, e.max) = (-12.0, 12.0);
                }
                return Some(e);
            }
        }
        None
    }

    fn eq_freqs(&self, api: &str, gains: &[&DevField]) -> Vec<f64> {
        let n = gains.len();
        if let Some(w) = self.apis.get(api).and_then(|a| a.write.as_ref()) {
            for st in &w.steps {
                match st.transform.as_str() {
                    "builtin:av6x02_eq_gains" => return vec![64.0, 180.0, 500.0, 1400.0, 3900.0, 11000.0],
                    "builtin:arctis5_eq_gains" => return vec![62.5, 250.0, 1000.0, 3600.0, 12000.0],
                    _ => {}
                }
            }
        }
        let fr: Vec<f64> = gains.iter().filter_map(|g| g.name.strip_prefix("band_").and_then(|v| v.parse().ok())).collect();
        if fr.len() == n {
            return fr;
        }
        if n == 10 {
            return crate::devices::STD10.to_vec();
        }
        (0..n).map(|i| 32.0 * (16000.0f64 / 32.0).powf(i as f64 / (n.max(2) - 1) as f64)).collect()
    }

    /// Field values to write an equalizer curve.
    pub fn eq_values(&self, e: &DevEq, bands: &[f64]) -> HashMap<String, FVal> {
        let mut vals = HashMap::new();
        if e.kind == "graphic" {
            for (i, f) in e.fields.iter().enumerate() {
                vals.insert(f.clone(), FVal::Num(bands.get(i).map(|b| b.clamp(e.min, e.max)).unwrap_or(0.0)));
            }
            // other fields (e.g. the preset number) — the lowest allowed value
            for f in self.out_fields(&e.api) {
                if vals.contains_key(&f.name) || f.constant.is_some() {
                    continue;
                }
                let v = if f.range.len() == 2 { self.resolve_num(&f.range[0]).unwrap_or(0.0) } else { 0.0 };
                vals.insert(f.name.clone(), FVal::Num(v));
            }
            return vals;
        }
        vals.insert("preset_type".into(), FVal::Num(1.0));
        vals.insert("name".into(), FVal::Str("Arcmix".into()));
        for i in 0..10 {
            let p = format!("band{}_", i + 1);
            let g = bands.get(i).map(|b| b.clamp(e.min, e.max)).unwrap_or(0.0);
            vals.insert(format!("{p}frequency"), FVal::Num(crate::devices::STD10[i]));
            vals.insert(format!("{p}filter_type"), FVal::Num(1.0)); // peak
            vals.insert(format!("{p}gain"), FVal::Num(g));
            vals.insert(format!("{p}q_factor"), FVal::Num(1.41));
        }
        vals
    }
}

// ---------- labels in Ukrainian (translated in the window) ----------

pub const LABEL_UA: &[(&str, &str)] = &[
    ("sidetone", "Чутність власного голосу"),
    ("side_tone", "Чутність власного голосу"),
    ("sidetone_gain", "Чутність власного голосу"),
    ("amplitude", "Рівень власного голосу"),
    ("enabled", "Увімкнено"),
    ("gain", "Підсилення"),
    ("mic_volume", "Гучність мікрофона (у гарнітурі)"),
    ("microphone_gain", "Підсилення мікрофона"),
    ("level", "Рівень"),
    ("muted_mic_brightness", "Яскравість індикатора вимкненого мікрофона"),
    ("mute_mic_brightness", "Яскравість індикатора вимкненого мікрофона"),
    ("power_inactivity_timer", "Автовимкнення"),
    ("inactivity_timer", "Автовимкнення"),
    ("minutes", "Хвилин"),
    ("off_time", "Автовимкнення"),
    ("time", "Автовимкнення"),
    ("volume_limiter", "Обмежувач гучності"),
    ("bluetooth_startup", "Bluetooth при ввімкненні"),
    ("bt_power_default", "Bluetooth при ввімкненні"),
    ("bt_call_default", "Дзвінки Bluetooth"),
    ("bt_startup_mode", "Bluetooth при ввімкненні"),
    ("bt_call_muting", "Звук гри під час дзвінка"),
    ("bt_gamemute_pref", "Звук гри під час дзвінка"),
    ("wireless_mode", "Режим бездротового зв'язку"),
    ("audio_mode", "Режим звуку"),
    ("anc", "Шумозаглушення"),
    ("anc_level", "Рівень шумозаглушення"),
    ("transparency", "Прозорість (звук навколо)"),
    ("transparency_level", "Рівень прозорості"),
    ("transparency_anc", "Шумозаглушення / прозорість"),
    ("transparency_anc_mode", "Шумозаглушення / прозорість"),
    ("transparent_anc_mode", "Шумозаглушення / прозорість"),
    ("active_noise_cancellation", "Шумозаглушення"),
    ("transparent", "Рівень прозорості"),
    ("wear_sense", "Датчик носіння"),
    ("wear_sense_config", "Датчик носіння"),
    ("volume_24g", "Гучність 2,4 ГГц"),
    ("volume_bt", "Гучність Bluetooth"),
    ("line_out_mode", "Лінійний вихід"),
    ("oled_brightness", "Яскравість екрана"),
    ("oled_screen_brightness", "Яскравість екрана"),
    ("dim_timer", "Затемнення екрана"),
    ("high_gain", "Високе підсилення"),
    ("device_gain", "Високе підсилення"),
    ("home_screen_type", "Головний екран"),
    ("home_screen_view", "Головний екран"),
    ("home_screen_option", "Параметри головного екрана"),
    ("oled_screen_saver_timer", "Заставка через"),
    ("oled_screen_saver_mode", "Заставка"),
    ("screensaver_mode", "Заставка"),
    ("screen_brightness", "Яскравість екрана"),
    ("display_timeout", "Вимкнення екрана"),
    ("surround", "Об'ємний звук"),
    ("surround_sound", "Об'ємний звук"),
    ("state", "Увімкнено"),
    ("mic_agc", "Автопідсилення мікрофона"),
    ("mic_noise_gate", "Шумовий поріг мікрофона"),
    ("mic_noise_reduction", "Шумоподавлення мікрофона"),
    ("min_gain", "Шумоподавлення мікрофона"),
    ("drc", "Компресор"),
    ("ratio", "Співвідношення"),
    ("threshold", "Поріг"),
    ("dac_gain", "Підсилення ЦАП"),
    ("host_mode", "Режим хоста"),
    ("aux_mode", "Режим AUX"),
    ("tx_led", "Індикатор передавача"),
    ("connected", "Коли підключено"),
    ("disconnected", "Коли відключено"),
    ("mute_illumination", "Підсвічування вимкненого мікрофона"),
    ("option", "Режим"),
    ("range_extend", "Розширений радіус"),
    ("sample_rate", "Частота"),
    ("boom_mic_sidetone", "Власний голос (мікрофон-штанга)"),
    ("on_ear_sidetone", "Власний голос (вбудований мікрофон)"),
    ("mode", "Режим"),
    ("volume", "Гучність"),
    ("value", "Значення"),
    ("dim", "Затемнення"),
];

pub const VALUE_UA: &[(&str, &str)] = &[
    ("off", "Вимк."),
    ("on", "Увімк."),
    ("low", "Низький"),
    ("medium", "Середній"),
    ("high", "Високий"),
    ("never", "Ніколи"),
    ("auto", "Авто"),
    ("steady", "Постійно"),
    ("breathing", "Пульсує"),
    ("solid", "Постійно"),
    ("enabled", "Увімк."),
    ("disabled", "Вимк."),
    ("anc", "Шумозаглушення"),
    ("transparency", "Прозорість"),
    ("transparent", "Прозорість"),
    ("stereo", "Стерео"),
    ("speed", "Швидкість"),
    ("range", "Дальність"),
    ("mute", "Вимкнути звук"),
    ("muted", "Без звуку"),
    ("lower", "Тихіше"),
    ("lower_volume", "Тихіше"),
    ("none", "Нічого"),
    ("max", "Максимум"),
    ("min", "Мінімум"),
    ("attenuate", "Тихіше"),
    ("attenuate_12db", "Тихіше на 12 дБ"),
    ("lower_12db", "Тихіше на 12 дБ"),
    ("bass_boost", "Більше басу"),
    ("chatmix", "Гра й чат"),
    ("custom", "Власний"),
    ("default", "Типово"),
    ("detailed", "Детально"),
    ("dim", "Тьмяно"),
    ("enter", "Увімкнути"),
    ("exit", "Вимкнути"),
    ("flat", "Рівний"),
    ("focus", "Фокус"),
    ("fps_footsteps", "Кроки (FPS)"),
    ("meters", "Індикатори"),
    ("preset", "Пресет"),
    ("simple", "Простий"),
    ("smiley", "Музика (V)"),
    ("speaker", "Динаміки"),
    ("stream", "Стрім"),
];

fn lookup(t: &'static [(&'static str, &'static str)], k: &str) -> Option<&'static str> {
    t.iter().find(|(a, _)| *a == k).map(|(_, b)| *b)
}

const GAME_NAMES: &[(&str, &str)] = &[
    ("apex_legends", "Apex Legends"),
    ("baldurs_gate_3", "Baldur's Gate 3"),
    ("cod_mwii", "CoD: MW II"),
    ("cod_warzone_2", "Warzone 2"),
    ("destiny_2", "Destiny 2"),
    ("diablo_iv", "Diablo IV"),
    ("fortnite", "Fortnite"),
    ("gta_v", "GTA V"),
    ("minecraft", "Minecraft"),
    ("overwatch_2", "Overwatch 2"),
    ("pubg", "PUBG"),
    ("rainbow_six_siege", "Rainbow Six Siege"),
    ("rocket_league", "Rocket League"),
];

fn preset_label(raw: &str) -> String {
    match GAME_NAMES.iter().find(|(k, _)| *k == raw.to_lowercase()) {
        Some((_, g)) => g.to_string(),
        None => value_label(raw),
    }
}

fn humanize(s: &str) -> String {
    let s = s.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

pub fn setting_label(api: &str, field: &str) -> String {
    if let Some(l) = lookup(LABEL_UA, api) {
        if field == api || field.chars().count() <= 7 || ["value", "level", "state", "mode", "minutes", "enabled", "gain"].contains(&field) {
            if field == api || ["value", "level", "minutes", "mode", "gain"].contains(&field) {
                return l.into();
            }
            if let Some(fl) = lookup(LABEL_UA, field) {
                if fl != l {
                    return format!("{l}: {}", fl.to_lowercase());
                }
            }
            return l.into();
        }
    }
    if let Some(l) = lookup(LABEL_UA, field) {
        return l.into();
    }
    humanize(field)
}

pub fn value_label(v: &str) -> String {
    let k = v.trim().to_lowercase();
    if let Some(l) = lookup(VALUE_UA, &k) {
        return l.into();
    }
    if k.ends_with(" minutes") || k.ends_with(" min") {
        return format!("{} хв", k.split_whitespace().next().unwrap_or(""));
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> HashMap<String, DevConfig> {
        parse_devices(include_str!("../../../web/devices.json")).unwrap().into_iter().map(|d| (d.file.clone(), d)).collect()
    }

    fn h(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn arctis_7_plus() {
        let all = all();
        let d = &all["arctis_7_plus.yaml"];
        let mut vals: HashMap<String, FVal> = (1..=10).map(|i| (format!("gain{i}"), FVal::Num(0.0))).collect();
        vals.insert("gain1".into(), FVal::Num(3.5));
        vals.insert("gain10".into(), FVal::Num(-12.0));
        let acts = d.prepare_write("eq", &vals).unwrap();
        let got = h(&acts[0].payload[..14]);
        let want = format!("0033{}{}0000", "1f", "18".repeat(8));
        assert_eq!(&got[..22], &want[..22], "eq payload");
        assert!(d.prepare_write("sidetone", &HashMap::from([("level".into(), FVal::Num(2.0))])).is_ok());
        assert!(d.prepare_read("headset_status").is_ok());
        let resp: Vec<u8> = [0x00, 0xb0, 0x03, 0x03, 0x03, 0x64, 0x64].into_iter().chain(std::iter::repeat_n(0, 58)).collect();
        let f = d.deserialize("headset_status", &resp).unwrap();
        assert_eq!(f.get("battery_level").map(|v| v.num()), Some(3.0));
        let r = d.dispatch(&[0x00, 0x45, 0x5a, 0x64]).expect("0x45 dispatch");
        assert!(r.emit.is_some());
        assert!(d.detect_eq().is_some());
    }

    /// The ChatMix map of a read as percentages: (game, chat).
    fn dial(d: &DevConfig, api: &str, resp: &[u8]) -> (f64, f64) {
        let e = d.sync_read.iter().find(|e| e.strukt == api).unwrap();
        let f = d.deserialize(api, resp).unwrap();
        let em = d.map_read(e, &f).into_iter().find(|m| m.signal == "chatmix_changed").unwrap();
        let v = |role: &str| em.fields.iter().find(|(k, _)| em.roles.get(*k).is_some_and(|r| r == role)).map(|(_, v)| v.num().round()).unwrap();
        (v("chatmix_game"), v("chatmix_chat"))
    }

    /// Arctis 7: interface 5, the dial as HeadsetControl reads it (0 — full volume, 191..255).
    #[test]
    fn arctis_7_quirks() {
        let all = all();
        let d = &all["arctis_7.yaml"];
        assert_eq!(d.device.hid.command.as_ref().map(|i| i.interface), Some(5));
        assert_eq!(d.device.hid.sync.as_ref().map(|i| i.interface), Some(5));
        let resp = |g: u8, c: u8| -> Vec<u8> { [0x06, 0x24, g, c].into_iter().chain(std::iter::repeat_n(0, 27)).collect() };
        assert_eq!(dial(d, "game_chat_status", &resp(0, 0)), (100.0, 100.0), "the middle");
        assert_eq!(dial(d, "game_chat_status", &resp(0, 191)), (100.0, 0.0), "all game");
        assert_eq!(dial(d, "game_chat_status", &resp(223, 0)), (50.0, 100.0), "towards chat");
        assert_eq!(dial(d, "game_chat_status", &resp(255, 0)), (100.0, 100.0));
    }

    /// Arctis 9: each channel 0..19.
    #[test]
    fn arctis_9_quirks() {
        let all = all();
        let d = &all["arctis_9.yaml"];
        let mut resp = vec![0u8; 65];
        (resp[10], resp[11]) = (19, 0);
        let (game, chat) = dial(d, "info_table", &resp);
        assert_eq!((game, chat), (0.0, 100.0));
        // the battery: 0x9A — full, charging
        (resp[4], resp[5]) = (0x9a, 1);
        let e = d.sync_read.iter().find(|e| e.strukt == "info_table").unwrap();
        let ems = d.map_read(e, &d.deserialize("info_table", &resp).unwrap());
        let b = ems.iter().find(|m| m.signal == "battery_changed").unwrap();
        assert_eq!(b.fields["bat_voltage_lo"].num().round(), 100.0);
        assert_eq!(b.roles["bat_voltage_lo"], "battery_headset");
        let c = ems.iter().find(|m| m.signal == "charging_changed").unwrap();
        assert_eq!(c.fields["bat_status"], FVal::Str("CHARGING".into()));
        assert!(!all["arctis_5.yaml"].device.capabilities.iter().any(|x| x == "wireless"), "Arctis 5 is wired");
    }

    /// Nova 7 Gen 2: the parametric curve as Arctis Sound Manager sends it (hardware_eq.py, checked against
    /// a Nova 7P Gen 2 there): a name frame, a band frame with the connection byte (frequency u16 LE, type,
    /// gain i8 in tenths of a dB, Q u16 LE in thousandths), then the commit frame.
    #[test]
    fn nova_7_gen2_parametric() {
        let all = all();
        let d = &all["nova_7_gen2.yaml"];
        let e = d.detect_eq().expect("parametric eq");
        assert_eq!((e.kind.as_str(), e.bands, e.min, e.max), ("parametric", 10, -12.0, 12.0));
        let mut curve = vec![0.0; 10];
        (curve[0], curve[5], curve[9]) = (12.0, -12.0, 3.5);
        let acts = d.prepare_write(&e.api, &d.eq_values(&e, &curve)).unwrap();
        let frames: Vec<&[u8]> = acts.iter().filter(|a| a.sleep.is_zero()).map(|a| a.payload.as_slice()).collect();
        assert_eq!(frames.len(), 3, "name, bands, commit");
        assert_eq!(h(&frames[0][..4]), "00a70001", "name: report, 0xA7, wired, custom preset");
        assert!(frames[0][4..].starts_with(b"Arcmix"));
        assert_eq!(h(&frames[1][..3]), "003300", "bands: report, 0x33, connection");
        let band = |i: usize| h(&frames[1][3 + i * 6..3 + i * 6 + 6]);
        assert_eq!(band(0), format!("2000{}{}{}", "01", "78", "8205"), "32 Hz peak +12 dB Q 1.41");
        assert_eq!(band(5), format!("e803{}{}{}", "01", "88", "8205"), "1 kHz peak -12 dB");
        assert_eq!(band(9), format!("803e{}{}{}", "01", "23", "8205"), "16 kHz peak +3.5 dB");
        assert_eq!(h(&frames[2][..2]), "0027", "commit");
        assert!(frames.iter().all(|f| f.len() == 65));
    }

    /// Every write and read of every description can be prepared; equalizers too.
    #[test]
    fn all_devices() {
        let all = all();
        assert!(!all.is_empty());
        let mut errs = Vec::new();
        for (n, d) in &all {
            for (api, a) in &d.apis {
                if a.write.is_none() {
                    continue;
                }
                let mut vals = HashMap::new();
                for f in d.out_fields(api) {
                    if f.constant.is_some() {
                        continue;
                    }
                    let v = if f.range.len() == 2 { d.resolve_num(&f.range[0]).unwrap_or(0.0) } else { 0.0 };
                    let x = match f.typ.as_str() {
                        "varstring" => FVal::Str("x".into()),
                        "bytearray" => FVal::Bytes(vec![0; f.size.max(0) as usize]),
                        _ if f.repeat > 1 => FVal::Arr(vec![FVal::Num(0.0); f.repeat as usize]),
                        _ => FVal::Num(v),
                    };
                    vals.insert(f.name.clone(), x);
                }
                if let Err(e) = d.prepare_write(api, &vals) {
                    errs.push(format!("{n}: {api}: {e}"));
                }
            }
            for e in &d.sync_read {
                if let Err(err) = d.prepare_read(&e.strukt) {
                    errs.push(format!("{n}: read {}: {err}", e.strukt));
                }
            }
            if let Some(eq) = d.detect_eq() {
                if let Err(e) = d.prepare_write(&eq.api, &d.eq_values(&eq, &vec![0.0; eq.bands])) {
                    errs.push(format!("{n}: eq write: {e}"));
                }
            }
            let _ = d.settings_schema();
        }
        assert!(errs.is_empty(), "{}", errs.join("\n"));
    }

    /// Model codes for recognition — the same as in the full descriptions.
    #[test]
    fn device_pids_match() {
        let src = include_str!("../../../web/devices.json");
        let pids = device_pids(src).unwrap();
        let mut n = 0;
        for d in parse_devices(src).unwrap() {
            for v in &d.device.variants {
                n += 1;
                assert!(pids.contains(&v.product_id), "no PID {:04x} ({})", v.product_id, d.device.name);
            }
        }
        assert!(n > 0);
    }

    /// All headset setting labels have translations.
    #[test]
    fn labels_translated() {
        let all = crate::i18n::all_strings();
        for (_, v) in LABEL_UA.iter().chain(VALUE_UA.iter()) {
            if *v == "ChatMix" {
                continue;
            }
            assert!(all.contains_key(*v), "no translation: {v:?}");
        }
    }
}
