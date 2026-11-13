# Edifier M60 BLE control protocol (reverse-engineered)

Decoded from Android HCI snoop captures of the official EDIFIER ConneX app.
Not affiliated with or endorsed by Edifier. Status labels: **confirmed** = seen in a capture
and matched to a known action; **inferred** = consistent with the data but not independently
confirmed; **unknown** = seen, meaning not established.

## Connecting

- The speaker advertises over BLE with **no name**: 16-bit service `0xF600` and manufacturer
  id `0x07E0`. The GAP name "EDIFIER BLE" is only readable after connecting. Find it by
  advertisement, not name. (macOS hides hardware addresses, so do not rely on one.)
- GATT service `4809f601-1a48-11e9-ab14-d663bd873d93`
  - notify: `48090001-1a48-11e9-ab14-d663bd873d93` (speaker -> host), subscribe first
  - write (without response): `48090002-1a48-11e9-ab14-d663bd873d93` (host -> speaker)
- One BLE client at a time. No client-side keepalive is needed (the speaker sends its own
  `0x84`/`0x86` status frames about every 19 s, alternating, i.e. one of them roughly every 9.5 s).
- Observed advertising behaviour: once a BLE client has connected, the speaker keeps
  advertising and accepts new connections in any input, including AUX, for at least 14 minutes.
  If no BLE client has connected, switching to AUX stops the advertising. Not tested: power
  cycle, and the exact conditions that stop advertising.

## Frame

    AA EC <cmd> <len_hi> <len_lo> <payload...> <checksum>    host -> speaker
    BB EC <cmd> <len_hi> <len_lo> <payload...> <checksum>    speaker -> host

`checksum` = sum of all preceding bytes, low 8 bits (**confirmed**, 441 of 441 captured frames).

## Commands

| cmd | direction | payload | meaning | status |
|---|---|---|---|---|
| `C9` | get | - | device name, ASCII ("EDIFIER M60") | confirmed |
| `66` | get / notify | reply `10 <vol>` | volume. App slider is 0-16 and the first byte is `0x10` = 16 (the maximum); 16 was set in a capture | confirmed |
| `67` | set | `<vol>` | set volume; reply `10 <vol>` | confirmed |
| `61` | get / notify | reply `0F <input>` | current input | confirmed |
| `62` | set | `0F <input>`; reply `01` | set input: 01 Bluetooth, 02 USB, 03 AUX | confirmed |
| `D5` | get / notify | reply `<preset>` | current EQ preset | confirmed |
| `C4` | set | `<preset>`; reply echoes | set EQ preset: 00 Music, 01 Monitor, 02 Game, 03 Movie, 04 Custom | inferred (ids from action order) |
| `43` | get | - ; reply `03 06 <profile>` | read the speaker's custom EQ profile | confirmed |
| `46` | set | `<profile>`; reply echoes | write the custom EQ profile | confirmed (format and sequence from the capture), not yet sent by M60 Tray to a real speaker |
| `48` | get | reply `<value>` | HD audio codec setting; 01 before, 02 after choosing 96 kHz | inferred |
| `49` | set | `<value>` | set HD audio codec. The speaker **drops Bluetooth immediately** | confirmed (disconnect) |
| `50` | notify only | `00 <title len> <artist len> <title> <artist>` (UTF-8) | now playing, pushed by the speaker: on connecting while on Bluetooth, on track changes (confirmed on the speaker) and on input changes. A source that plays without sending track info (a game on a phone) also shows up as `Not Provided`, so it cannot be told apart from nothing playing. `Not Provided` (artist empty) when the source reports nothing; five zero bytes after switching away from Bluetooth. No play/pause state. Never requested by the app | confirmed (captures: a podcast title and artist, `Not Provided`, zeros) |
| `68` | get / notify | 1 byte | seen 00, 04, 05 (05 in AUX, 04 after the codec change) | unknown |
| `C3` | get | reply `03` | constant in both captures | unknown |
| `C6` | get | reply `02 04 01` | firmware version, bytes are major, minor, patch (app shows 2.4.1) | confirmed |
| `D8` | get | 22 bytes | device info / capabilities | unknown |
| `84` | get / notify | reply `03 <duration> <sensitivity>` | Intelligent Backlight. duration: 00 5 s, 01 10 s, 02 20 s, 03 On. sensitivity: 00 High, 01 General, 02 Low | confirmed |
| `85` | set | `03 <duration> <sensitivity>`; reply echoes | set Intelligent Backlight (both fields every time) | confirmed |
| `86` | get / notify | reply `02 <v>` | Prompts & Tones, Bluetooth connection tone: 01 on, 00 off | confirmed |
| `87` | set | `02 01 <v>`; reply echoes | set the Bluetooth connection tone | confirmed |
| `C2` | set | `<action>`, no reply | AVRCP media action: 00 Play, 01 Pause, 04 Next, 05 Previous. Taken from kralonur/edifier-speakers-ble; not in our captures | from other docs; user reports it only works on the Bluetooth input (USB untested) |
| `CE` | set | none, no reply | **power off**. The speaker drops the link ~2 s later and cannot be woken over BLE | confirmed |

Not yet captured: auto power-off and any other settings.

## Custom EQ profile (`46` payload / `43` reply minus its first 2 bytes)

    6 band records, 6 bytes each:  <index> 00 <freq_hi> <freq_lo> <gain> 07
    4-byte profile id: the profile's creation time, little-endian Unix seconds
    name, UTF-8, no terminator

- Bands seen: 62, 250, 1000, 4000, 8000, 16000 Hz.
- `gain = 6 + 2 * dB` (0 dB = 6, 0.5 dB steps). The app's sliders run -3 to +3 dB, i.e. bytes 0..12.
  Verified at -3, -1, 0, 0.5 and 1 dB against the values shown in the app for two profiles.
- The speaker holds **one** custom profile (preset 04). The app sends whichever saved profile
  you pick; the other saved profiles live in the app, not on the speaker.
- The ids in the captures decode to 2026-11-10 11:05:54 and 11:08:42 UTC, three minutes apart,
  which matches when the two profiles were made: the id is the creation time.
- The `43` reply is `03 06 <profile>`; `06` matches the band count.
- **Applying a custom preset**, as the phone app does it: `C4 04` (select Custom), then `43`
  (read the current profile), then `46 <profile>` (the app sent it twice). Switching between custom
  presets while already on Custom is just `46 <profile>`. M60 Tray sends `C4 04` (only when the
  EQ is not already Custom) followed by one `46`; its tests check those bytes against the capture.
- The trailing `07` byte in each band record (unknown meaning) and the leading `03` in the `43`
  reply (unknown) were constant in both captures.
- The built-in presets (Music, Monitor, Game, Movie) are selected by id only; their curves are
  never sent over BLE, so they cannot be read from the speaker or the capture.

See `crates/m60-core/src/proto.rs` for `build_frame`, `parse_frame`, `decode_custom_eq` and `encode_custom_eq`.

## Cross-check with kralonur/edifier-speakers-ble

An independent write-up of the same protocol (https://github.com/kralonur/edifier-speakers-ble,
derived from the ConneX 1.0.30 Android app, tested on firmware 2.4.1) agrees with everything in the
tables above: UUIDs, frame layout, checksum, volume 0-16, input and EQ ids, EQ gain mapping, smart
light and tone values, and the meaning of `68` (00 = USB 48 kHz / standard Bluetooth, 04 = 96 kHz
LDAC active, 05 = USB 96 kHz or AUX), `C6` (firmware) and `C9` (name).

Additional opcodes that document describes and that we have **not** verified ourselves:

| cmd | meaning | notes |
|---|---|---|
| `CA` | set device name | UTF-8, 35-byte limit in the app |
| `44` | set one custom EQ band | `46` (whole profile) is what our captures show |
| `47` | write custom EQ date/name | |
| `C8` | Classic Bluetooth address | unit specific |
| `CD` | disconnect A2DP | no reply; may drop the whole connection |
| `F2` | headset state | unverified on M60 |

`C2` (media action) is implemented in the app; we use the frames published there
(`AA EC C2 00 01 <action> <checksum>`). In practice it only does something while the speaker is on the
Bluetooth input, so the app disables the media buttons on other inputs.
