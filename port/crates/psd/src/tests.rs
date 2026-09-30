use super::*;
use image::{Luma, Rgba};

/// A strict reader for what `write` produces, following the same steps as Compositor's
/// PSDReader.swift (field order, Pascal name padding, additional info lengths and PSB long keys),
/// and additionally checking every length adds up to the end of the file.
struct Cursor<'a> {
    d: &'a [u8],
    o: usize,
}

impl<'a> Cursor<'a> {
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        assert!(self.o + n <= self.d.len(), "read of {n} bytes at {} runs past the end ({})", self.o, self.d.len());
        let s = &self.d[self.o..self.o + n];
        self.o += n;
        s
    }
    fn u8(&mut self) -> u8 {
        self.bytes(1)[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.bytes(2).try_into().unwrap())
    }
    fn i16(&mut self) -> i16 {
        self.u16() as i16
    }
    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.bytes(4).try_into().unwrap())
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.bytes(8).try_into().unwrap())
    }
    fn len(&mut self, psb: bool) -> usize {
        if psb { self.u64() as usize } else { self.u32() as usize }
    }
    fn key(&mut self) -> [u8; 4] {
        self.bytes(4).try_into().unwrap()
    }
}

#[derive(Debug)]
struct ParsedLayer {
    rect: [i32; 4],
    channels: Vec<(i16, usize)>,
    blend: [u8; 4],
    opacity: u8,
    clipping: u8,
    flags: u8,
    mask: Option<([i32; 4], u8, u8)>,
    legacy_name: Vec<u8>,
    extras: Vec<([u8; 4], Vec<u8>)>,
    planes: Vec<(i16, u16, Vec<u8>)>,
}

impl ParsedLayer {
    fn extra(&self, key: &[u8; 4]) -> Option<&[u8]> {
        self.extras.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice())
    }
    fn name(&self) -> String {
        let p = self.extra(b"luni").expect("luni");
        let count = u32::from_be_bytes(p[..4].try_into().unwrap()) as usize;
        let units: Vec<u16> = (0..count).map(|i| u16::from_be_bytes([p[4 + i * 2], p[5 + i * 2]])).collect();
        String::from_utf16(&units).unwrap()
    }
    fn section(&self) -> Option<u32> {
        self.extra(b"lsct").map(|p| u32::from_be_bytes(p[..4].try_into().unwrap()))
    }
    fn plane(&self, id: i16) -> &[u8] {
        &self.planes.iter().find(|p| p.0 == id).unwrap().2
    }
}

#[derive(Debug)]
struct Parsed {
    version: u16,
    channels: u16,
    width: u32,
    height: u32,
    depth: u16,
    mode: u16,
    resolution: f64,
    raw_count: i16,
    layers: Vec<ParsedLayer>,
    merged: Vec<Vec<u8>>,
}

fn decode(data: &[u8], compression: u16, w: usize, h: usize, psb: bool) -> Vec<u8> {
    match compression {
        0 => {
            assert_eq!(data.len(), w * h);
            data.to_vec()
        }
        1 => {
            let mut c = Cursor { d: data, o: 0 };
            let counts: Vec<usize> = (0..h).map(|_| if psb { c.u32() as usize } else { c.u16() as usize }).collect();
            let mut out = Vec::new();
            for n in counts {
                out.extend(unpack_bits(c.bytes(n), w).unwrap());
            }
            assert_eq!(c.o, data.len(), "RLE channel has trailing bytes");
            out
        }
        2 => inflate_stored(data),
        other => panic!("compression {other}"),
    }
}

fn inflate_stored(data: &[u8]) -> Vec<u8> {
    assert_eq!(&data[..2], &[0x78, 0x01]);
    let mut c = Cursor { d: data, o: 2 };
    let mut out = Vec::new();
    loop {
        let header = c.u8();
        assert_eq!(header & 6, 0, "stored blocks only");
        let len = u16::from_le_bytes(c.bytes(2).try_into().unwrap());
        let nlen = u16::from_le_bytes(c.bytes(2).try_into().unwrap());
        assert_eq!(len, !nlen);
        out.extend_from_slice(c.bytes(len as usize));
        if header & 1 == 1 {
            break;
        }
    }
    assert_eq!(c.u32(), zlib::adler32(&out));
    assert_eq!(c.o, data.len());
    out
}

fn parse(d: &[u8]) -> Parsed {
    let mut c = Cursor { d, o: 0 };
    assert_eq!(c.bytes(4), b"8BPS");
    let version = c.u16();
    let psb = version == 2;
    assert_eq!(c.bytes(6), &[0; 6]);
    let channels = c.u16();
    let height = c.u32();
    let width = c.u32();
    let depth = c.u16();
    let mode = c.u16();
    let color_data = c.u32() as usize;
    c.bytes(color_data);
    let resources_len = c.u32() as usize;
    let resources_end = c.o + resources_len;
    let mut resolution = 72.0;
    while c.o + 12 <= resources_end {
        assert_eq!(c.bytes(4), b"8BIM");
        let id = c.u16();
        let name_len = c.u8() as usize;
        c.bytes(name_len);
        if (name_len + 1) % 2 == 1 {
            c.u8();
        }
        let len = c.u32() as usize;
        let start = c.o;
        if id == 1005 {
            resolution = c.u32() as f64 / 65536.0;
        }
        c.o = start + len + len % 2;
    }
    assert_eq!(c.o, resources_end);

    let section_len = c.len(psb);
    let section_end = c.o + section_len;
    let mut layers = Vec::new();
    let mut raw_count = 0;
    if section_len > 0 {
        let info_len = c.len(psb);
        let info_end = c.o + info_len;
        assert_eq!(info_len % 2, 0, "layer info length is even");
        raw_count = c.i16();
        for _ in 0..raw_count.unsigned_abs() {
            layers.push(parse_record(&mut c, psb));
        }
        for layer in &mut layers {
            for &(id, len) in &layer.channels.clone() {
                let data = c.bytes(len);
                let compression = u16::from_be_bytes([data[0], data[1]]);
                let (w, h) = if id == -2 {
                    let m = layer.mask.unwrap().0;
                    ((m[3] - m[1]) as usize, (m[2] - m[0]) as usize)
                } else {
                    ((layer.rect[3] - layer.rect[1]) as usize, (layer.rect[2] - layer.rect[0]) as usize)
                };
                let plane = if w * h == 0 {
                    assert_eq!(len, 2, "an empty channel is only its compression code");
                    Vec::new()
                } else {
                    decode(&data[2..], compression, w, h, psb)
                };
                layer.planes.push((id, compression, plane));
            }
        }
        if c.o % 2 == 1 || c.o < info_end {
            c.bytes(info_end - c.o);
        }
        assert_eq!(c.o, info_end, "layer info length matches its contents");
        let global_mask = c.u32() as usize;
        c.bytes(global_mask);
        assert_eq!(c.o, section_end, "layer and mask section length matches its contents");
    }

    let compression = c.u16();
    let bytes_per_sample = depth as usize / 8;
    let (row, rows) = (width as usize * bytes_per_sample, height as usize);
    let merged = match compression {
        0 => (0..channels).map(|_| c.bytes(row * rows).to_vec()).collect(),
        1 => {
            let counts: Vec<usize> = (0..channels as usize * rows).map(|_| if psb { c.u32() as usize } else { c.u16() as usize }).collect();
            let packed: Vec<Vec<u8>> = counts.iter().map(|&n| unpack_bits(c.bytes(n), row).unwrap()).collect();
            packed.chunks(rows).map(|rs| rs.concat()).collect()
        }
        other => panic!("merged compression {other}"),
    };
    assert_eq!(c.o, d.len(), "the file ends right after the image data");
    Parsed { version, channels, width, height, depth, mode, resolution, raw_count, layers, merged }
}

fn parse_record(c: &mut Cursor, psb: bool) -> ParsedLayer {
    let rect = [c.i32(), c.i32(), c.i32(), c.i32()];
    let n = c.u16();
    let channels = (0..n).map(|_| (c.i16(), c.len(psb))).collect();
    assert_eq!(c.bytes(4), b"8BIM");
    let blend = c.key();
    let opacity = c.u8();
    let clipping = c.u8();
    let flags = c.u8();
    assert_eq!(c.u8(), 0);
    let extra_len = c.u32() as usize;
    let extra_end = c.o + extra_len;
    let mask_len = c.u32() as usize;
    let mask_end = c.o + mask_len;
    let mask = (mask_len >= 20).then(|| ([c.i32(), c.i32(), c.i32(), c.i32()], c.u8(), c.u8()));
    c.o = mask_end;
    let ranges = c.u32() as usize;
    assert_eq!(ranges, 40);
    c.bytes(ranges);
    let name_len = c.u8() as usize;
    let legacy_name = c.bytes(name_len).to_vec();
    c.bytes((4 - (name_len + 1) % 4) % 4);
    let mut extras = Vec::new();
    while c.o + 12 <= extra_end {
        assert_eq!(c.bytes(4), b"8BIM");
        let key = c.key();
        let len = if psb && PSB_LONG_KEYS.contains(&&key) { c.u64() as usize } else { c.u32() as usize };
        assert_eq!(len % 2, 0, "additional info length is even");
        extras.push((key, c.bytes(len).to_vec()));
    }
    assert_eq!(c.o, extra_end, "extra data length matches its contents");
    ParsedLayer { rect, channels, blend, opacity, clipping, flags, mask, legacy_name, extras, planes: Vec::new() }
}

fn noise(w: u32, h: u32, seed: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let v = (x * 7 + y * 13 + seed * 31) % 256;
        // Flat stretches as well as noise, so RLE has both repeats and literals.
        let r = if x < w / 3 { 200 } else { v as u8 };
        Rgba([r, (v * 3 % 256) as u8, (x * 255 / w.max(1)) as u8, if y % 5 == 0 { 255 } else { (v / 2) as u8 }])
    })
}

fn sample_doc() -> Document {
    let mut doc = Document::new(40, 30);
    doc.resolution = 300.0;
    doc.push(Layer::pixels("Background", 0, 0, noise(40, 30, 1)));
    let mask = GrayImage::from_fn(10, 8, |x, _| Luma([(x * 25) as u8]));
    doc.push(Group::new(
        "Outer",
        vec![
            Layer::pixels("Off canvas", -10, 20, noise(25, 15, 2)).opacity(128).fill(64).blend(Blend::Multiply).into(),
            Layer::pixels("Clipped", 5, 5, noise(6, 6, 3)).clipped().hidden().into(),
            Group::new("Inner", vec![Layer::pixels("Ünïcödé 🎨", 3, 4, noise(9, 7, 4)).mask(Mask::new(1, 2, mask).default_color(0).relative_to_layer().disabled()).into()])
                .blend(Blend::Screen)
                .opacity(200)
                .closed()
                .into(),
        ],
    ));
    doc.push(Layer::adjustment("Levels", Adjustment::Levels(Levels { records: [LevelsRecord { input_black: 10, input_white: 240, output_black: 5, output_white: 250, gamma: 1.25 }, LevelsRecord::IDENTITY, LevelsRecord::IDENTITY, LevelsRecord::IDENTITY] })));
    doc
}

#[test]
fn pack_bits_round_trips() {
    let mut rows: Vec<Vec<u8>> = vec![vec![], vec![7], vec![1, 1], vec![1, 2], vec![0; 300], (0..=255).collect(), (0..1000).map(|i| (i / 3 % 7) as u8).collect()];
    for len in [127, 128, 129, 255, 256, 257] {
        rows.push(vec![9; len]);
        rows.push((0..len).map(|i| (i * 37 % 251) as u8).collect());
        rows.push((0..len).map(|i| if i % 4 < 2 { 1 } else { (i % 256) as u8 }).collect());
    }
    for row in rows {
        let packed = pack_bits(&row);
        assert!(packed.len() <= row.len() + row.len().div_ceil(128));
        assert_eq!(unpack_bits(&packed, row.len()).unwrap(), row);
    }
    // Photoshop's example from the TIFF PackBits description, including a -128 no-op.
    let packed = [0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0x80, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7, 0xAA];
    let expected = [0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA];
    assert_eq!(unpack_bits(&packed, expected.len()).unwrap(), expected);
    assert!(unpack_bits(&[0x05, 1, 2], 6).is_err());
}

fn check_structure(version: Version, compression: Compression) {
    let doc = sample_doc();
    let options = WriteOptions { version, compression, unicode_names: true };
    let bytes = write(&doc, &options).unwrap();
    let p = parse(&bytes);
    assert_eq!(p.version, if version == Version::Psb { 2 } else { 1 });
    assert_eq!((p.channels, p.width, p.height, p.depth, p.mode), (4, 40, 30, 8, 3));
    assert_eq!(p.resolution, 300.0);
    // Background, divider, Off canvas, Clipped, divider, Unicode, Inner, Outer, Levels.
    assert_eq!(p.raw_count, -9);
    let names: Vec<String> = p.layers.iter().map(|l| l.name()).collect();
    assert_eq!(names, ["Background", "</Layer group>", "Off canvas", "Clipped", "</Layer group>", "Ünïcödé 🎨", "Inner", "Outer", "Levels"]);
    let sections: Vec<Option<u32>> = p.layers.iter().map(|l| l.section()).collect();
    assert_eq!(sections, [None, Some(3), None, None, Some(3), None, Some(2), Some(1), None]);

    let bg = &p.layers[0];
    assert_eq!(bg.rect, [0, 0, 30, 40]);
    assert_eq!(bg.channels.iter().map(|c| c.0).collect::<Vec<_>>(), [-1, 0, 1, 2]);
    let img = noise(40, 30, 1);
    for (id, c) in [(0, 0), (1, 1), (2, 2), (-1, 3)] {
        assert_eq!(bg.plane(id), img.as_raw().chunks(4).map(|p| p[c]).collect::<Vec<_>>());
    }
    let code = compression.code();
    assert!(bg.planes.iter().all(|p| p.1 == code));

    let off = &p.layers[2];
    assert_eq!(off.rect, [20, -10, 35, 15]);
    assert_eq!((&off.blend, off.opacity, off.flags & 2), (b"mul ", 128, 0));
    assert_eq!(off.extra(b"iOpa").unwrap(), [64, 0, 0, 0]);
    let clipped = &p.layers[3];
    assert_eq!((clipped.clipping, clipped.flags & 2), (1, 2));
    assert!(clipped.extra(b"iOpa").is_none());

    let uni = &p.layers[5];
    assert_eq!(uni.legacy_name, b"?n?c?d? ?");
    let (rect, default, flags) = uni.mask.unwrap();
    assert_eq!((rect, default, flags), ([2, 1, 10, 11], 0, 3));
    assert_eq!(uni.plane(-2), GrayImage::from_fn(10, 8, |x, _| Luma([(x * 25) as u8])).as_raw().as_slice());

    let inner = &p.layers[6];
    assert_eq!((&inner.blend, inner.opacity, inner.rect), (b"scrn", 200, [0; 4]));
    assert_eq!(inner.extra(b"lsct").unwrap(), [0, 0, 0, 2, b'8', b'B', b'I', b'M', b's', b'c', b'r', b'n']);
    assert!(inner.planes.iter().all(|p| p.2.is_empty()));
    let outer = &p.layers[7];
    assert_eq!(&outer.blend, b"pass");
    assert_eq!(&outer.extra(b"lsct").unwrap()[8..], b"pass");
    assert_eq!(p.layers[1].extra(b"lsct").unwrap(), [0, 0, 0, 3]);

    let levels = p.layers[8].extra(b"levl").unwrap();
    assert_eq!(levels.len(), 292);
    assert_eq!(&levels[..12], &[0, 2, 0, 10, 0, 240, 0, 5, 0, 250, 0, 125]);

    assert_eq!(p.merged.len(), 4);
    assert!(p.merged.iter().all(|m| m.len() == 40 * 30));
}

#[test]
fn psd_rle_structure() {
    check_structure(Version::Psd, Compression::Rle);
}

#[test]
fn psd_raw_structure() {
    check_structure(Version::Psd, Compression::Raw);
}

#[test]
fn psd_zip_structure() {
    check_structure(Version::Psd, Compression::Zip);
}

#[test]
fn psb_rle_structure() {
    check_structure(Version::Psb, Compression::Rle);
}

#[test]
fn psb_raw_structure() {
    check_structure(Version::Psb, Compression::Raw);
}

#[test]
fn psb_uses_long_lengths() {
    let mut doc = Document::new(8, 4);
    doc.push(Layer::pixels("A", 0, 0, noise(8, 4, 9)));
    let psd = write(&doc, &WriteOptions::psd()).unwrap();
    let psb = write(&doc, &WriteOptions::psb()).unwrap();
    // Section and layer info lengths (+4 +4), four channel lengths (+16), and 4-byte RLE row
    // counts for the layer's four channels and the composite's four (+2 per row).
    assert_eq!(psb.len() - psd.len(), 4 + 4 + 16 + 2 * 4 * 4 + 2 * 4 * 4);
    // An `Lr16` block is one of the keys with an 8-byte length in a PSB, `luni` isn't.
    let doc = {
        let mut d = Document::new(2, 2);
        d.push(Layer::empty("x").extra(*b"Lr16", vec![1, 2, 3]));
        d
    };
    let p = parse(&write(&doc, &WriteOptions::psb()).unwrap());
    assert_eq!(p.layers[0].extra(b"Lr16").unwrap(), [1, 2, 3, 0]);
}

#[test]
fn adjustment_payloads() {
    let curves = Adjustment::Curves(Curves { channels: [Some(vec![(0, 0), (128, 160), (255, 255)]), None, Some(vec![(0, 20), (255, 230)]), None] });
    assert_eq!(
        curves.payload(),
        [0, 0, 1, 0, 0, 0, 0b101, 0, 3, 0, 0, 0, 0, 0, 160, 0, 128, 0, 255, 0, 255, 0, 2, 0, 20, 0, 0, 0, 230, 0, 255]
    );
    let hue = Adjustment::HueSaturation(HueSaturation { colorize: true, colorize_values: [200, 60, -10], ..Default::default() }).payload();
    assert_eq!(hue.len(), 100);
    assert_eq!(&hue[..10], &[0, 2, 1, 0, 0, 200, 0, 60, 0xff, 0xf6]);
    assert_eq!(&hue[16..24], &[1, 59, 1, 89, 0, 15, 0, 45]); // Reds: 315, 345, 15, 45.
    let levels = Adjustment::Levels(Levels::default()).payload();
    assert_eq!(levels.len(), 292);
    assert!(levels[2..].chunks(10).all(|r| r == [0, 0, 0, 255, 0, 0, 0, 255, 0, 100]));
}

#[test]
fn composite_is_normal_over_white_matted() {
    let mut doc = Document::new(3, 1);
    doc.push(Layer::pixels("Opaque", 0, 0, RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 255]))));
    doc.push(Layer::pixels("Half", 1, 0, RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]))).opacity(128));
    doc.push(Layer::pixels("Hidden", 0, 0, RgbaImage::from_pixel(3, 1, Rgba([255, 0, 0, 255]))).hidden());
    let p = parse(&write(&doc, &WriteOptions::psd()).unwrap());
    assert_eq!(p.merged[3], [255, 128, 0]);
    assert_eq!(p.merged[0], [10, 127, 255]);
    assert_eq!(p.merged[2], [30, 127, 255]);
}

#[test]
fn flat_files() {
    let (w, h) = (5u32, 3u32);
    let plane: Vec<u16> = (0..w * h).map(|i| (i * 4000) as u16).collect();
    let image = FlatImage { width: w, height: h, mode: ColorMode::Grayscale, depth: 16, planes: vec![plane.clone()], resolution: 72.0 };
    for options in [WriteOptions::psd(), WriteOptions::psd().compression(Compression::Raw), WriteOptions::psb()] {
        let p = parse(&write_flat(&image, &options).unwrap());
        assert_eq!((p.channels, p.depth, p.mode, p.raw_count), (1, 16, 1, 0));
        assert!(p.layers.is_empty());
        let samples: Vec<u16> = p.merged[0].chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
        assert_eq!(samples, plane);
    }
    let cmyk = FlatImage { width: 2, height: 2, mode: ColorMode::Cmyk, depth: 8, planes: vec![vec![255; 4]; 4], resolution: 72.0 };
    let p = parse(&write_flat(&cmyk, &WriteOptions::psd()).unwrap());
    assert_eq!((p.channels, p.mode), (4, 4));
    assert!(write_flat(&FlatImage { mode: ColorMode::Indexed, ..cmyk }, &WriteOptions::psd()).is_err());
}

#[test]
fn rejects_oversized_documents() {
    assert!(write(&Document::new(30_001, 1), &WriteOptions::psd()).is_err());
    assert!(write(&Document::new(0, 1), &WriteOptions::psd()).is_err());
}
