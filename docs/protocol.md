# TEPRA PRO SR3700P Printer Protocol

This document describes the proprietary print job format used by the
KING JIM TEPRA PRO SR3700P label printer, as reverse engineered for the
Rust implementation in this repository. It is the result of capturing
the raw bytes that Windows' vendor driver sends to the printer, without
any access to official KING JIM documentation.

All findings here are empirical: derived by diffing several captured
print jobs against each other (different text, font sizes, and tape
widths) and, for the USB transport layer, by testing directly against
real hardware. Fields whose meaning could not be determined are called
out explicitly.

## 1. Transport: USB

### 1.1 Device identification

```
$ lsusb -d 0d8a:0103
Bus 001 Device 059: ID 0d8a:0103 King Jim Co., Ltd SR3700P
```

- Vendor ID: `0x0d8a` (King Jim Co., Ltd)
- Product ID: `0x0103` (SR3700P)
- USB 1.10, Full Speed (12 Mbps), bus-powered (100 mA)

### 1.2 USB descriptors

The device exposes a single configuration with a single interface
using the standard **USB Printer class**:

```
Interface Descriptor:
  bInterfaceClass         7   (Printer)
  bInterfaceSubClass      1   (Printer)
  bInterfaceProtocol      2   (Bidirectional)
  bNumEndpoints           2
  Endpoint 0x81  IN,  Bulk, wMaxPacketSize=64
  Endpoint 0x02  OUT, Bulk, wMaxPacketSize=64
```

The standard USB Printer class `GET_DEVICE_ID` control request
(`bmRequestType=0xA1, bRequest=0`) returns:

```
<MFG:KING JIM;CMD:ESCPL2;MDL:TEPRA PRO SR3700P;CLS:PRINTER;
```

Note the `CMD:ESCPL2` field — the vendor calls its command language
"ESCPL2", though it bears no relation to Epson's ESC/P or ESC/P2.

Because the interface advertises the standard Printer class, Linux's
kernel automatically binds the `usblp` driver to it. Any userspace
program that wants to talk to the printer directly over
`libusb`/`rusb` must ask libusb to detach the kernel driver first
(`set_auto_detach_kernel_driver(true)` before `claim_interface`),
otherwise `claim_interface` fails with "Resource busy".

### 1.3 Sending a job

**Sending a print job is a single USB bulk write.** The full job byte
stream (see section 2) is written directly to the bulk OUT endpoint
(`0x02`), with **no additional framing, no handshake, and no response
expected on the bulk IN endpoint**. libusb/the OS automatically split
the write into 64-byte USB transactions as needed; there is no need to
chunk the write manually.

```rust
handle.write_bulk(0x02, &job_bytes, timeout)?;
```

This was confirmed by printing successfully from Linux with exactly
this sequence (see `examples/probe.rs` and `src/usb/mod.rs`).

### 1.4 A dead end: USB Mass Storage Bulk-Only Transport

An earlier capture of the **Windows** driver's USB traffic (via
USBPcap) showed what looked exactly like a USB Mass Storage
Bulk-Only Transport (BOT) exchange: 31-byte Command Block Wrappers
starting with the `USBC` signature, 13-byte Command Status Wrappers
starting with `USBS`, and SCSI-like opcodes (`TEST UNIT READY` =
`0x00`, `REQUEST SENSE` = `0x03`) used to poll whether tape was
loaded.

This looked like a plausible transport (and would explain why the
printer shows up as a "disk-like" device to some tools), so an initial
implementation replicated this BOT protocol faithfully. **It did not
work**: sending a `TEST UNIT READY` CBW to the real printer over Linux
resulted in the printer never responding on the bulk IN endpoint at
all (confirmed both with `rusb` and with Python's `pyusb`, ruling out
a Rust/library-specific bug).

Once real device probing revealed the actual USB Printer class
descriptors and the `CMD:ESCPL2`/`CLS:PRINTER` device ID string, the
BOT hypothesis was abandoned in favor of just writing the raw job
bytes to the bulk OUT endpoint directly — which worked immediately.
The `USBC`/`USBS` exchange seen in the Windows capture remains
unexplained (possibly an artifact of how the Windows class driver or
a status-monitoring utility polls tape presence through some other
mechanism entirely unrelated to submitting the job), but is not
required to print.

### 1.5 Detecting tape width

The TEPRA desktop app has a "detect tape width" feature that queries
the printer for the currently loaded tape's width, rather than relying
on the user to select it manually. This is a single vendor-specific
USB control transfer, captured via USBPcap on Windows
(`captures/obtain-tape-width.pcap`) and confirmed against real
hardware for three tape widths:

```
bmRequestType = 0xC1  (Device-to-Host, Vendor, Interface)
bRequest      = 0x01
wValue        = 0x0000
wIndex        = 0x0000
wLength       = 8
```

The response is always 8 bytes long. Byte 3 (0-indexed) is a tape
width *index*, not the width in mm directly:

| Tape loaded | Response bytes (hex)      | Byte 3 (index) |
|------------:|----------------------------|:---------------:|
| 6mm         | `08 00 00 01 00 00 00 00`  | 1                |
| 12mm        | `08 00 00 03 00 00 00 00`  | 3                |
| 18mm        | `08 00 00 04 00 00 00 00`  | 4                |

These three index values (1, 3, 4) match exactly the position of each
width in KING JIM's official TEPRA tape width lineup for this printer
line, `[4, 6, 9, 12, 18, 24, 36, 50]` mm (0-indexed: 4mm→0, 6mm→1,
9mm→2, 12mm→3, 18mm→4, 24mm→5, 36mm→6, 50mm→7; per KING JIM's product
site). This strongly suggests the full mapping is:

| Index | Tape width |
|:-----:|-----------:|
| 0     | 4mm        |
| 1     | 6mm        |
| 2     | 9mm        |
| 3     | 12mm       |
| 4     | 18mm       |
| 5     | 24mm       |
| 6     | 36mm       |
| 7     | 50mm       |

Indices 0, 2, 5, 6, and 7 (4mm, 9mm, 24mm, 36mm, 50mm) are
extrapolated from the lineup pattern and have **not** been confirmed
against real hardware or a capture — only 6mm, 12mm, and 18mm are
verified (which also happen to be the only three widths this crate's
`TapeWidth` currently implements end-to-end, i.e. with known
`width_byte`/`aux_byte`/raster parameters from section 3).

The meaning of the other 7 bytes in the response (always `08 00 00
<idx> 00 00 00 00` in every observed sample) is unknown; byte 0 (`08`)
is constant and does not vary with tape width, and might just be an
echo of `wLength`. No sample with no tape loaded at all has been
captured, so it's unknown whether/how that case is distinguished from
a real width index.

## 2. Job byte stream format

A complete print job is a flat byte stream with four parts, back to
back, in this order:

```
+----------------+------------------------+------------------+--------+
| Header block   | Configuration commands | Raster rows (xN) | Footer |
| (fixed 2179 B) | C, D, G, L, T           | one per column   |        |
+----------------+------------------------+------------------+--------+
```

### 2.1 Common framing: `ESC { }` command frames

Most parts of the protocol other than the raster rows use a common
"command frame" wrapper:

```
0x1B 0x7B <len> <cmd> <params...> <checksum> 0x7D
 ESC  '{'                                     '}'
```

- `cmd` is a single ASCII byte identifying the command (`C`, `D`, `G`,
  `L`, `T`, `@`, or — for the header block only — a fixed marker byte
  `0x80` that does not follow this rule, see 2.2).
- `len` is the number of bytes from `cmd` (inclusive) through the
  checksum (inclusive), i.e. `1 (cmd) + params.len() + 1 (checksum)`.
- `checksum` is the sum of `cmd` and all `params` bytes, truncated to
  a `u8` (`sum mod 256`). This checksum formula is used consistently
  by every frame in the protocol, including the large header block
  (section 2.2).

Implemented in `src/tepra/frame.rs`.

### 2.2 Header block

The header block is a single large frame, structurally similar to the
command frames above but with a fixed length marker instead of a
computed one:

```
0x1B 0x7B 0x80 <2174 bytes of payload> <checksum> 0x7D
```

It is always exactly 2179 bytes (`2 + 1 + 2174 + 1 + 1`). Diffing
several captured jobs (varying tape width, font size, and character
count) shows the block is almost entirely constant. Known fields
(offsets are relative to the start of the frame, i.e. including the
leading `ESC {` and the `0x80` length byte):

| Offset | Field                        | Notes |
|-------:|-------------------------------|-------|
| 0–2    | `1B 7B 80`                     | Frame start + fixed length marker |
| 8      | Tape width in mm               | `0x06`=6mm, `0x0C`=12mm, `0x12`=18mm |
| 0x21   | `"SPC10"` (UTF-16LE)            | Unknown purpose; possibly a format/protocol version tag |
| 0x61   | `"306"` (UTF-16LE)              | Unknown purpose |
| 0x81   | `"obj_barcode\t\nprint_form\tGeneral\ncatalog_desc\t"` (ASCII) | Unknown purpose; looks like leftover template/object metadata from the desktop app's internal document model |
| 20     | Grows by 4 per extra character  | `value = 12 + 4 * char_count`. Confirmed for char_count 1 and 2; likely related to some per-character/per-object table size, but the underlying table itself has not been located. |
| 23     | Tape-width-dependent byte, unknown meaning | 6mm→3, 12mm→5, 18mm→6. No formula found relating it to tape width or bitmap size (see also 3.1) — kept as a hardcoded lookup table. |
| 2177   | Checksum                       | `sum(bytes[3..2177]) mod 256` |
| 2178   | `0x7D`                          | Frame end |

Everything else in the 2174-byte payload has been observed to be
constant (mostly zero-filled) across every capture taken so far and is
kept verbatim as a template (`assets/header_18mm_template.bin`); only
the fields above are patched before recomputing the checksum.

Implemented in `src/tepra/header.rs`.

### 2.3 Configuration commands

Five `ESC { }` command frames follow the header block, always in this
order:

| Cmd | Params (hex)         | Meaning |
|-----|----------------------|---------|
| `C` | `03 01 01 01`         | Unknown. Constant in every capture. |
| `D` | `05`                  | Unknown. Constant in every capture. |
| `G` | *(none)*              | Unknown. Constant in every capture. |
| `L` | `<u32 LE>`            | Number of raster rows that follow. |
| `T` | `<u16 LE>`            | Number of leading blank raster rows (the print content's offset from the start of the raster data — see 3.2). |

Example (from a captured 36pt "A" on 18mm tape, 170 total raster
rows, content starting at row 36):

```
1B 7B 07 43 03 01 01 01 49 7D   ESC{ len=7 'C' 03 01 01 01 checksum=0x49 }
1B 7B 04 44 05 49 7D            ESC{ len=4 'D' 05          checksum=0x49 }
1B 7B 03 47 47 7D               ESC{ len=3 'G'             checksum=0x47 }
1B 7B 07 4C AA 00 00 00 F6 7D   ESC{ len=7 'L' 170 (LE32)   checksum=0xF6 }
1B 7B 05 54 24 00 78 7D         ESC{ len=5 'T' 36  (LE16)   checksum=0x78 }
```

Implemented in `src/tepra/builder.rs` (frame ordering/assembly) and
`src/tepra/frame.rs` (frame encoding).

### 2.4 Raster rows

After the configuration commands, exactly `L` raster rows follow back
to back, with **no separators and no per-row checksum**:

```
0x1B 0x2E 0x00 0x0D 0x0D 0x01 <width_byte> <bitmap: N bytes>
 ESC  '.'  <-- fixed 4 bytes -->  ^ tape-width dependent
```

Each row corresponds to one column of dots along the tape feed
direction; the bits within a row's bitmap correspond to dot positions
across the tape's width. Rendering text is therefore done glyph
column by glyph column, with **no explicit rotation needed**: a
glyph's X axis maps directly to the row index (feed direction) and its
Y axis maps directly to the bit position within a row (tape width
direction). This was confirmed by reconstructing readable "A" and "B"
glyphs pixel-for-pixel from captured jobs.

The 4 constant bytes after `ESC .` (`00 0D 0D 01`) are unknown; they
have been observed identical in every capture regardless of tape
width, font size, or content.

`width_byte` and the bitmap length `N` depend on the tape width — see
section 3.1 for the exact relationship. Bits are packed MSB-first.

Implemented in `src/tepra/raster.rs`.

### 2.5 Footer

After the last raster row:

```
0x0C  1B 7B 03 40 40 <checksum> 7D
      \_______________ ESC{ len=3 '@' checksum='@' '}' _______________/
```

- A single `0x0C` byte (unknown purpose; constant in every capture).
- A final `ESC { }` frame with command `@` (0x40) and no parameters
  (its "checksum" therefore just equals `0x40` again, since the sum of
  a single `0x40` byte mod 256 is `0x40`).

Implemented as `TRAILING_BYTE` + `build_command_frame(b'@', &[])` in
`src/tepra/builder.rs`.

## 3. Tape width parameters

Three tape widths have been captured and verified byte-for-byte: 6mm,
12mm, and 18mm.

### 3.1 Raster row width byte and bitmap size

| Tape width | Header byte (offset 8) | Row `width_byte` | Bitmap size |
|-----------:|:-----------------------:|:-----------------:|:-----------:|
| 6mm        | `0x06`                  | `0x38` (56)        | 8 bytes (64 bits) |
| 12mm       | `0x0C`                  | `0x70` (112)       | 15 bytes (120 bits) |
| 18mm       | `0x12`                  | `0xA8` (168)       | 22 bytes (176 bits) |

These three data points fit two exact formulas:

```
width_byte   = width_mm * 28 / 3
bitmap_bits  = width_byte + 8
bitmap_bytes = ceil(bitmap_bits / 8)
```

Note that the bitmap size is **not** simply proportional to
`width_mm` (that would predict 6mm/12mm/18mm to be in a 1:2:3 ratio,
but 8/15/22 is not). The `width_byte * 28/3` relationship was only
discovered after capturing the 12mm sample — with only 6mm and 18mm
data points, a naive linear fit through the origin looked plausible
but was wrong.

This formula is unverified for tape widths other than 6/12/18mm;
support for other widths (3.5mm, 9mm, 24mm, etc., which the printer
line is known to support) would need additional captures to confirm
or refute it.

### 3.2 Header offset 23 ("aux byte")

| Tape width | Value |
|-----------:|------:|
| 6mm        | 3     |
| 12mm       | 5     |
| 18mm       | 6     |

No formula relating this value to `width_mm`, `width_byte`, or the
bitmap size has been found. It is stored as a hardcoded lookup table
(`TapeWidth::aux_byte` in `src/tepra/mod.rs`) pending further data
points.

## 4. Font size / point-to-pixel conversion

The TEPRA desktop app's point sizes (e.g. "36pt") do not map to
`fontdue`'s `px` (pixels-per-em) parameter 1:1. By rasterizing a
captured 36pt "A" glyph at various `px` values and comparing the
resulting bounding box width against the captured bitmap's column
span (63 dots, matching the feed-direction extent of the glyph), an
exact match was found at `px = 104`:

```
208 = 104 / 36 * 72   (dots per inch, "printer point size" -> px)
px  = pt * 208 / 72
```

This constant (`DOTS_PER_INCH = 208.0` in `src/font/mod.rs`) has only
been validated against a single glyph/size pairing (36pt "A"); it
should hold for other sizes assuming the relationship is linear
(which is the norm for TrueType/OpenType rasterization), but has not
been independently re-verified at other point sizes.

## 5. Open questions

The following aspects of the protocol remain unresolved. They do not
prevent basic text printing (all currently-known values are either
constant or have been reduced to a formula/lookup table), but would
need further captures or hardware experimentation to pin down:

- **`C`, `D`, `G` command parameters** (section 2.3): always constant
  across every capture taken (1–2 plain text characters, two font
  sizes, three tape widths). Likely control print density, cut
  behavior, or some other per-job setting that simply wasn't varied in
  any of the test captures.
- **Header block offset 20** (section 2.2): the `12 + 4 * char_count`
  formula is confirmed for 1 and 2 characters only; unverified for
  longer strings or non-text objects (e.g. barcodes, given the
  `obj_barcode` string embedded elsewhere in the header).
- **Header block offset 23 / "aux byte"** (section 3.2): no formula
  found from three data points.
- **Most of the 2174-byte header payload**: mostly zero-filled and
  treated as an opaque template; likely contains fields relevant to
  features not yet exercised (e.g. barcodes, multi-line labels,
  print density/quality settings, cut mode) given the `SPC10`, `306`,
  and `obj_barcode`/`print_form`/`catalog_desc` strings embedded in it.
- **The `USBC`/`USBS`-like exchange seen in the Windows USBPcap
  capture** (section 1.4): not required for printing, but its actual
  purpose (if any) on real hardware is unexplained.
- **Tape widths other than 6/12/18mm**: the formulas in section 3.1
  are extrapolated from exactly three data points and unverified
  beyond them.

## 6. References / raw data

- `tests/fixtures/*.prn` — raw job byte streams captured by redirecting
  the Windows driver's output to a `FILE:` port, for various text/tape
  width/font size combinations. Used as golden files in byte-exact
  reconstruction tests.
- `captures/obtain-tape-width.pcap` — USBPcap capture of the driver
  probing the tape width.
- `src/tepra/` — the Rust implementation described above.
- `examples/probe.rs` — opens the printer over USB and reports its
  device ID / tape info, without printing anything.
- `examples/send_raw.rs` — sends a raw job file (e.g. one produced via
  `--dump`, or one of the `tests/fixtures/*.prn` files above) directly
  to the printer, bypassing font rendering.
