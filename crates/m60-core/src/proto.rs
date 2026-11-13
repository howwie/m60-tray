//! Edifier M60 BLE protocol, decoded from Android HCI snoop captures of the official app.
//! See PROTOCOL.md for the evidence behind each command.
//!
//! Frame: `AA EC <cmd> <len_hi> <len_lo> <payload...> <checksum>`  host -> speaker
//!        `BB EC <cmd> <len_hi> <len_lo> <payload...> <checksum>`  speaker -> host
//! Checksum = sum of all preceding bytes, low 8 bits.

// ---- GATT -----------------------------------------------------------------
pub const SERVICE_UUID: &str = "4809f601-1a48-11e9-ab14-d663bd873d93";
/// Speaker -> host notifications (ATT handle 0x000c in the capture). Subscribe first.
pub const NOTIFY_UUID: &str = "48090001-1a48-11e9-ab14-d663bd873d93";
/// Host -> speaker, write without response (ATT handle 0x0010).
pub const WRITE_UUID: &str = "48090002-1a48-11e9-ab14-d663bd873d93";

/// 16-bit service 0xF600 in the advertisement, as a full 128-bit UUID.
pub const ADV_SERVICE: u128 = 0x0000f600_0000_1000_8000_00805f9b34fb;
pub const ADV_MANUFACTURER_ID: u16 = 0x07E0;

pub const REQ: u8 = 0xAA;
pub const RSP: u8 = 0xBB;
const MAGIC: u8 = 0xEC;

// ---- commands ---------------------------------------------------------------
pub const CMD_NAME: u8 = 0xC9;
pub const CMD_GET_VOLUME: u8 = 0x66; // reply / notification: <0x10> <volume>
pub const CMD_SET_VOLUME: u8 = 0x67; // payload <volume>; reply <0x10> <volume>
/// The app's slider is 0-16 and every volume reply starts with 0x10.
pub const DEFAULT_MAX_VOLUME: u8 = 16;

pub const CMD_INPUT_GET: u8 = 0x61; // reply / notification: <0x0F> <input>
pub const CMD_INPUT_SET: u8 = 0x62; // payload <0x0F> <input>
pub const INPUT_KEY: u8 = 0x0F;
pub const INPUT_BLUETOOTH: u8 = 1;

pub const CMD_EQ_GET: u8 = 0xD5; // reply / notification: <preset>
pub const CMD_EQ_SET: u8 = 0xC4; // payload <preset>; reply echoes it

/// The speaker holds ONE custom EQ profile, used while the EQ preset is [`EQ_CUSTOM`].
/// To apply a custom preset the phone app sends `C4 04` (select Custom) and then the whole
/// profile with `46`; switching between custom presets while on Custom is just another `46`.
pub const CMD_CUSTOM_EQ_GET: u8 = 0x43; // reply <03> <06 = band count> + profile
pub const CMD_CUSTOM_EQ_SET: u8 = 0x46; // payload = profile; echoed back
pub const EQ_CUSTOM: u8 = 4;

/// LDAC / HD audio preference. Changing it makes the speaker drop Bluetooth, so no UI uses it.
pub const CMD_CODEC_GET: u8 = 0x48;
pub const CMD_CODEC_SET: u8 = 0x49;

pub const CMD_BACKLIGHT_GET: u8 = 0x84; // reply <03> <duration> <sensitivity>
pub const CMD_BACKLIGHT_SET: u8 = 0x85; // payload <03> <duration> <sensitivity>
pub const BACKLIGHT_KEY: u8 = 0x03;

pub const CMD_PROMPT_GET: u8 = 0x86; // reply <02> <0|1>
pub const CMD_PROMPT_SET: u8 = 0x87; // payload <02> <01> <0|1>
pub const PROMPT_KEY: u8 = 0x02;
pub const PROMPT_BT_CONNECTION: u8 = 0x01;

/// AVRCP media action (play/pause/next/previous). From kralonur/edifier-speakers-ble; not in
/// our own captures. Payload is one action byte; there is no reply.
pub const CMD_MEDIA: u8 = 0xC2;

/// Now playing, pushed by the speaker (never asked for): when a client connects while on the
/// Bluetooth input, when the track changes, and when the input changes. Payload:
/// `<00> <title length> <artist length> <title UTF-8> <artist UTF-8>`. With nothing to report the
/// title is "Not Provided"; after switching away from Bluetooth the payload is five zero bytes.
/// It does not say whether the source is playing or paused.
pub const CMD_NOW_PLAYING: u8 = 0x50;

/// Power off. No payload, no reply; the link drops ~2 s later and BLE cannot wake the speaker.
pub const CMD_POWER_OFF: u8 = 0xCE;

/// Firmware version: reply `<major> <minor> <patch>`, e.g. 02 04 01 = 2.4.1.
pub const CMD_FIRMWARE_GET: u8 = 0xC6;

// ---- value tables --------------------------------------------------------------
pub type Table = &'static [(u8, &'static str)];

pub const INPUTS: Table = &[(1, "Bluetooth"), (2, "USB"), (3, "AUX")];
/// Preset ids come from the order of the actions in the capture, confirmed by kralonur's docs.
pub const EQ_PRESETS: Table = &[(0, "Music"), (1, "Monitor"), (2, "Game"), (3, "Movie"), (4, "Custom")];
pub const BACKLIGHT_DURATIONS: Table = &[(0, "5 s"), (1, "10 s"), (2, "20 s"), (3, "On")];
pub const BACKLIGHT_SENSITIVITIES: Table = &[(0, "High"), (1, "General"), (2, "Low")];
pub const MEDIA_ACTIONS: Table = &[(0x00, "Play"), (0x01, "Pause"), (0x04, "Next"), (0x05, "Previous")];

pub fn lookup(table: Table, key: u8) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

pub fn is_valid(table: Table, key: u8) -> bool {
    lookup(table, key).is_some()
}

// ---- frames -----------------------------------------------------------------------
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

pub fn build_frame(cmd: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= u16::MAX as usize, "payload too long");
    let mut f = Vec::with_capacity(payload.len() + 6);
    f.extend_from_slice(&[REQ, MAGIC, cmd]);
    f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    f.extend_from_slice(payload);
    f.push(checksum(&f));
    f
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u8,
    pub payload: Vec<u8>,
}

/// Parse exactly one complete speaker frame.
pub fn parse_frame(raw: &[u8]) -> Option<Frame> {
    if raw.len() < 6 || raw[0] != RSP || raw[1] != MAGIC {
        return None;
    }
    let n = u16::from_be_bytes([raw[3], raw[4]]) as usize;
    if raw.len() != 6 + n || checksum(&raw[..raw.len() - 1]) != raw[raw.len() - 1] {
        return None;
    }
    Some(Frame { cmd: raw[2], payload: raw[5..5 + n].to_vec() })
}

/// Reassembles speaker frames from notification chunks, so a frame split across two
/// notifications (small MTU) still parses, and garbage between frames is skipped.
#[derive(Default)]
pub struct FrameBuffer {
    buf: Vec<u8>,
}

const MAX_PAYLOAD: usize = 512;

impl FrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            // find the start of a frame
            let start = self.buf.windows(2).position(|w| w[0] == RSP && w[1] == MAGIC);
            match start {
                Some(i) => {
                    self.buf.drain(..i);
                }
                None => {
                    // keep a trailing 0xBB: its 0xEC may arrive in the next chunk
                    let keep = self.buf.last() == Some(&RSP);
                    self.buf.clear();
                    if keep {
                        self.buf.push(RSP);
                    }
                    break;
                }
            }
            if self.buf.len() < 5 {
                break;
            }
            let n = u16::from_be_bytes([self.buf[3], self.buf[4]]) as usize;
            if n > MAX_PAYLOAD {
                self.buf.drain(..1); // not a real header, resync
                continue;
            }
            let total = 6 + n;
            if self.buf.len() < total {
                break; // wait for the rest
            }
            match parse_frame(&self.buf[..total]) {
                Some(f) => {
                    out.push(f);
                    self.buf.drain(..total);
                }
                None => {
                    self.buf.drain(..1);
                }
            }
        }
        out
    }
}

// ---- now playing ----------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
}

/// Decode a `0x50` payload. `Some(None)` means "nothing to show" (no track info, or not on
/// Bluetooth); `None` means the payload did not have the expected shape.
pub fn decode_now_playing(payload: &[u8]) -> Option<Option<NowPlaying>> {
    if payload.len() < 3 {
        return None;
    }
    if payload.iter().all(|b| *b == 0) {
        return Some(None);
    }
    let (tl, al) = (usize::from(payload[1]), usize::from(payload[2]));
    if 3 + tl + al > payload.len() {
        return None;
    }
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
    let title = text(&payload[3..3 + tl]);
    let artist = text(&payload[3 + tl..3 + tl + al]);
    if (title.is_empty() || title == "Not Provided") && artist.is_empty() {
        return Some(None);
    }
    Some(Some(NowPlaying { title, artist }))
}

// ---- advertisement matching -------------------------------------------------------------
/// How well an advertisement looks like an M60: 2 = both markers, 1 = one, 0 = neither.
pub fn match_score(services: &[u128], manufacturer_ids: &[u16]) -> u8 {
    let svc = services.contains(&ADV_SERVICE);
    let mfr = manufacturer_ids.contains(&ADV_MANUFACTURER_ID);
    match (svc, mfr) {
        (true, true) => 2,
        (false, false) => 0,
        _ => 1,
    }
}

// ---- custom EQ profile (not wired to any UI yet) ------------------------------------------
// 6 band records of 6 bytes: <index> <00> <freq_hi> <freq_lo> <gain> <07>, then a 4-byte
// profile id (looks like a little-endian Unix timestamp), then the UTF-8 name.
// gain byte = 6 + 2*dB; the app's sliders run -3..+3 dB, i.e. bytes 0..12.
pub const BAND_COUNT: usize = 6;
const BAND_RECORD: usize = 6;
/// Band centre frequencies, as the app sends them.
pub const BAND_FREQS_HZ: [u16; BAND_COUNT] = [62, 250, 1000, 4000, 8000, 16000];
pub const GAIN_MIN_DB: f32 = -3.0;
pub const GAIN_MAX_DB: f32 = 3.0;
pub const GAIN_STEP_DB: f32 = 0.5;
/// Longest profile name we send, in UTF-8 bytes (the app's own names are short; the real limit is unknown).
pub const MAX_PROFILE_NAME: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub index: u8,
    pub zero: u8,
    pub freq_hz: u16,
    pub gain_db: f32,
    pub tail: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomEq {
    pub bands: Vec<Band>,
    pub profile_id: [u8; 4],
    pub name: String,
}

/// Round to the nearest 0.5 dB step inside the app's -3..+3 dB range.
pub fn snap_gain(db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    ((db.clamp(GAIN_MIN_DB, GAIN_MAX_DB) / GAIN_STEP_DB).round() * GAIN_STEP_DB) + 0.0
    // + 0.0 turns -0 into 0
}

/// Trim and cut a profile name to [`MAX_PROFILE_NAME`] bytes on a character boundary.
pub fn clean_profile_name(name: &str) -> String {
    let name = name.trim();
    let mut end = name.len().min(MAX_PROFILE_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].trim_end().to_string()
}

impl CustomEq {
    /// A profile in the layout the app sends: fixed frequencies, gains snapped to 0.5 dB steps,
    /// `profile_id` = creation time as a little-endian Unix timestamp (that is what the app uses).
    pub fn new(name: &str, gains_db: [f32; BAND_COUNT], profile_id: u32) -> Self {
        let bands = BAND_FREQS_HZ
            .iter()
            .zip(gains_db)
            .enumerate()
            .map(|(i, (f, g))| Band { index: i as u8, zero: 0, freq_hz: *f, gain_db: snap_gain(g), tail: 0x07 })
            .collect();
        CustomEq { bands, profile_id: profile_id.to_le_bytes(), name: clean_profile_name(name) }
    }

    pub fn id(&self) -> u32 {
        u32::from_le_bytes(self.profile_id)
    }

    pub fn gains_db(&self) -> Vec<f32> {
        self.bands.iter().map(|b| b.gain_db).collect()
    }
}

/// Decode the payload of a `0x43` reply: `<03> <band count> <profile...>`.
pub fn decode_custom_eq_reply(payload: &[u8]) -> Option<CustomEq> {
    if payload.len() < 2 || usize::from(payload[1]) != BAND_COUNT {
        return None;
    }
    decode_custom_eq(&payload[2..])
}

pub fn gain_to_byte(db: f32) -> u8 {
    (6.0 + 2.0 * db).round().clamp(0.0, 12.0) as u8
}

pub fn byte_to_gain(b: u8) -> f32 {
    (f32::from(b) - 6.0) / 2.0
}

/// Decode the payload of a `0x46` request (or the `0x43` reply minus its first two bytes).
pub fn decode_custom_eq(profile: &[u8]) -> Option<CustomEq> {
    let head = BAND_COUNT * BAND_RECORD;
    if profile.len() < head + 4 {
        return None;
    }
    let bands = profile[..head]
        .as_chunks::<BAND_RECORD>()
        .0
        .iter()
        .map(|r| Band {
            index: r[0],
            zero: r[1],
            freq_hz: u16::from_be_bytes([r[2], r[3]]),
            gain_db: byte_to_gain(r[4]),
            tail: r[5],
        })
        .collect();
    let mut id = [0u8; 4];
    id.copy_from_slice(&profile[head..head + 4]);
    Some(CustomEq { bands, profile_id: id, name: String::from_utf8_lossy(&profile[head + 4..]).into_owned() })
}

pub fn encode_custom_eq(eq: &CustomEq) -> Vec<u8> {
    let mut out = Vec::new();
    for b in &eq.bands {
        out.extend_from_slice(&[b.index, b.zero]);
        out.extend_from_slice(&b.freq_hz.to_be_bytes());
        out.extend_from_slice(&[gain_to_byte(b.gain_db), b.tail]);
    }
    out.extend_from_slice(&eq.profile_id);
    out.extend_from_slice(eq.name.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Request frames copied from the btsnoop captures of the official app: (command, payload, raw).
    const CAPTURED: &[(u8, &str, &str)] = &[
        (0x43, "", "aaec430000d9"),
        (0x48, "", "aaec480000de"),
        (0x49, "02", "aaec49000102e2"),
        (0x61, "", "aaec610000f7"),
        (0x62, "0f01", "aaec6200020f010a"),
        (0x62, "0f02", "aaec6200020f020b"),
        (0x62, "0f03", "aaec6200020f030c"),
        (0x66, "", "aaec660000fc"),
        (0x67, "00", "aaec67000100fe"),
        (0x67, "07", "aaec6700010705"),
        (0x67, "0d", "aaec6700010d0b"),
        (0x67, "10", "aaec670001100e"),
        (0x68, "", "aaec680000fe"),
        (0x84, "", "aaec8400001a"),
        (0x85, "030001", "aaec85000303000122"),
        (0x85, "030100", "aaec85000303010022"),
        (0x85, "030101", "aaec85000303010123"),
        (0x85, "030102", "aaec85000303010224"),
        (0x85, "030201", "aaec85000303020124"),
        (0x85, "030301", "aaec85000303030125"),
        (0x86, "", "aaec8600001c"),
        (0x87, "020100", "aaec87000302010023"),
        (0x87, "020101", "aaec87000302010124"),
        (0xC3, "", "aaecc3000059"),
        (0xC4, "00", "aaecc40001005b"),
        (0xC4, "01", "aaecc40001015c"),
        (0xC4, "02", "aaecc40001025d"),
        (0xC4, "03", "aaecc40001035e"),
        (0xC4, "04", "aaecc40001045f"),
        (0xC6, "", "aaecc600005c"),
        (0xC9, "", "aaecc900005f"),
        (0xCE, "", "aaecce000064"),
        (0xD5, "", "aaecd500006b"),
        (0xD8, "", "aaecd800006e"),
    ];

    #[test]
    fn builder_reproduces_every_captured_request() {
        for (cmd, payload, raw) in CAPTURED {
            assert_eq!(build_frame(*cmd, &hex(payload)), hex(raw), "cmd {cmd:#04x} payload {payload}");
        }
    }

    #[test]
    fn media_frames_match_published_examples() {
        for (action, raw) in
            [(0u8, "aaecc200010059"), (1, "aaecc20001015a"), (4, "aaecc20001045d"), (5, "aaecc20001055e")]
        {
            assert_eq!(build_frame(CMD_MEDIA, &[action]), hex(raw));
        }
    }

    fn rsp(cmd: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![RSP, MAGIC, cmd];
        f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        f.extend_from_slice(payload);
        f.push(checksum(&f));
        f
    }

    #[test]
    fn parse_roundtrip_and_rejections() {
        let f = rsp(0x66, &[0x10, 7]);
        assert_eq!(parse_frame(&f), Some(Frame { cmd: 0x66, payload: vec![0x10, 7] }));
        let mut bad = f.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(parse_frame(&bad), None, "bad checksum");
        assert_eq!(parse_frame(&f[..f.len() - 1]), None, "truncated");
        assert_eq!(parse_frame(&build_frame(0x66, &[])), None, "request frames are not responses");
        assert_eq!(parse_frame(&[]), None);
    }

    #[test]
    fn buffer_handles_split_joined_and_noisy_input() {
        let a = rsp(0x66, &[0x10, 7]);
        let b = rsp(0x61, &[0x0F, 2]);
        let mut fb = FrameBuffer::new();
        // one frame split in two chunks
        assert!(fb.push(&a[..4]).is_empty());
        assert_eq!(fb.push(&a[4..]).len(), 1);
        // two frames in one chunk
        let both: Vec<u8> = a.iter().chain(&b).copied().collect();
        let got = fb.push(&both);
        assert_eq!(got.iter().map(|f| f.cmd).collect::<Vec<_>>(), vec![0x66, 0x61]);
        // garbage before, between and after, and a split magic pair
        let mut noisy = vec![0x00, 0xBB, 0x01];
        noisy.extend_from_slice(&a);
        noisy.extend_from_slice(&[0xFF, 0xFF]);
        let mut fb = FrameBuffer::new();
        assert_eq!(fb.push(&noisy).len(), 1);
        assert!(fb.push(&b[..1]).is_empty()); // just the 0xBB
        assert_eq!(fb.push(&b[1..]).len(), 1);
        // a corrupt frame is dropped and the next good one still parses
        let mut corrupt = a.clone();
        *corrupt.last_mut().unwrap() ^= 0xFF;
        let mut fb = FrameBuffer::new();
        let mut stream = corrupt;
        stream.extend_from_slice(&b);
        assert_eq!(fb.push(&stream).iter().map(|f| f.cmd).collect::<Vec<_>>(), vec![0x61]);
    }

    #[test]
    fn now_playing_decodes_the_captured_pushes() {
        // first capture: a podcast playing on the phone
        let p = hex("001e1b54686520686973746f7279206f66206c696768742028636c617373696329506c616e6574204d6f6e657920e280a22030342f31312f32303236");
        assert_eq!(
            decode_now_playing(&p),
            Some(Some(NowPlaying {
                title: "The history of light (classic)".into(),
                artist: "Planet Money \u{2022} 04/11/2026".into()
            }))
        );
        // nothing to report
        assert_eq!(decode_now_playing(&hex("000c004e6f742050726f7669646564")), Some(None));
        // after switching away from Bluetooth
        assert_eq!(decode_now_playing(&hex("0000000000")), Some(None));
        // malformed: lengths that run past the end, or too short
        assert_eq!(decode_now_playing(&hex("00ff00414243")), None);
        assert_eq!(decode_now_playing(&hex("0001")), None);
        // a title without an artist is still shown
        assert_eq!(
            decode_now_playing(&hex("0003004142430a")).map(|n| n.map(|n| n.title)),
            Some(Some("ABC".to_string()))
        );
    }

    #[test]
    fn match_score_rules() {
        assert_eq!(match_score(&[ADV_SERVICE], &[ADV_MANUFACTURER_ID]), 2);
        assert_eq!(match_score(&[ADV_SERVICE], &[]), 1);
        assert_eq!(match_score(&[], &[ADV_MANUFACTURER_ID]), 1);
        assert_eq!(match_score(&[1], &[2]), 0);
    }

    #[test]
    fn custom_eq_decodes_the_two_captured_profiles() {
        // 0x46 payloads from the capture (frame header and checksum removed)
        let music = hex("0000003e0707010000fa0807020003e8040703000fa0070704001f40060705003e80040792faf26a4d75736963");
        let eq = decode_custom_eq(&music).unwrap();
        assert_eq!(eq.name, "Music");
        let gains: Vec<f32> = eq.bands.iter().map(|b| b.gain_db).collect();
        assert_eq!(gains, vec![0.5, 1.0, -1.0, 0.5, 0.0, -1.0]); // what the user set in the app
        let freqs: Vec<u16> = eq.bands.iter().map(|b| b.freq_hz).collect();
        assert_eq!(freqs, vec![62, 250, 1000, 4000, 8000, 16000]);
        assert_eq!(encode_custom_eq(&eq), music);

        let gaming =
            hex("0000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e67");
        let eq = decode_custom_eq(&gaming).unwrap();
        assert_eq!(eq.name, "Gaming");
        let gains: Vec<f32> = eq.bands.iter().map(|b| b.gain_db).collect();
        assert_eq!(gains, vec![-3.0, -3.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(encode_custom_eq(&eq), gaming);
    }

    #[test]
    fn new_profiles_match_what_the_app_sent() {
        // The two profiles from the capture, rebuilt from the values the user set in the app.
        let music = CustomEq::new("Music", [0.5, 1.0, -1.0, 0.5, 0.0, -1.0], 0x6af2_fa92);
        assert_eq!(
            build_frame(CMD_CUSTOM_EQ_SET, &encode_custom_eq(&music)),
            hex("aaec46002d0000003e0707010000fa0807020003e8040703000fa0070704001f40060705003e80040792faf26a4d757369633e")
        );
        let gaming = CustomEq::new("Gaming", [-3.0, -3.0, 0.0, 0.0, 0.0, 0.0], 0x6af2_fb3a);
        assert_eq!(
            build_frame(CMD_CUSTOM_EQ_SET, &encode_custom_eq(&gaming)),
            hex("aaec46002e0000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e672e")
        );
        // the ids are the creation times: 2026-11-10 11:05:54 and 11:08:42 UTC
        assert_eq!((music.id(), gaming.id()), (1_794_308_754, 1_794_308_922));
    }

    #[test]
    fn the_43_reply_decodes() {
        // captured reply payload: 03 06 + the Gaming profile
        let p = hex("03060000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e67");
        let eq = decode_custom_eq_reply(&p).unwrap();
        assert_eq!(eq.name, "Gaming");
        assert_eq!(eq.gains_db(), vec![-3.0, -3.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(decode_custom_eq_reply(&p[..10]), None, "too short");
        let mut wrong = p.clone();
        wrong[1] = 5;
        assert_eq!(decode_custom_eq_reply(&wrong), None, "unexpected band count");
    }

    #[test]
    fn gains_snap_and_names_are_cleaned() {
        assert_eq!(snap_gain(1.26), 1.5);
        assert_eq!(snap_gain(1.24), 1.0);
        assert_eq!(snap_gain(-9.0), -3.0);
        assert_eq!(snap_gain(9.0), 3.0);
        assert_eq!(snap_gain(f32::NAN), 0.0);
        assert_eq!(snap_gain(-0.1).to_bits(), 0.0f32.to_bits(), "no negative zero");
        assert_eq!(clean_profile_name("  Late night  "), "Late night");
        assert_eq!(clean_profile_name("abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnopqrst");
        assert_eq!(clean_profile_name("ééééééééééééééé").len(), 20, "cut on a character boundary");
    }

    #[test]
    fn gain_byte_range() {
        assert_eq!(gain_to_byte(-3.0), 0);
        assert_eq!(gain_to_byte(3.0), 12);
        assert_eq!(gain_to_byte(9.0), 12, "clamped to the app's range");
        assert_eq!(gain_to_byte(-9.0), 0);
        assert_eq!(byte_to_gain(6), 0.0);
    }
}
